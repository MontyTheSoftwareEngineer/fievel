use crate::{
    config::Hints,
    detect::{self, DetectionLimits, DetectionTrace, Outcome, Rect},
    font::{Canvas, Color},
    label::{AppendResult, LabelAlphabet, LabelSelection},
};
use evdev::KeyCode as K;
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fs::{memfd_create, MemfdFlags};
use std::{
    error::Error,
    fs::File,
    io,
    os::{fd::AsFd, unix::fs::FileExt},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::{
    layer_shell::v1::client::{
        zwlr_layer_shell_v1::{self, Layer},
        zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
    },
    screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1},
    virtual_pointer::v1::client::{zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(3);
const INPUT_TIMEOUT: Duration = Duration::from_secs(1);
const HINT_NAMESPACE: &str = "fievel-hints";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickKind {
    Left,
    Right,
}

#[derive(Debug)]
pub enum HintResult {
    Continue,
    Cancelled,
    Clicked,
}

#[derive(Clone, Debug)]
struct HintRegion {
    output_index: usize,
    rect: Rect,
    label: String,
}

#[derive(Default)]
struct HintBackground {
    hidden: bool,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum DebugView {
    #[default]
    Normal,
    Edges,
    Components,
}

impl DebugView {
    fn next(self) -> Self {
        match self {
            Self::Normal => Self::Edges,
            Self::Edges => Self::Components,
            Self::Components => Self::Normal,
        }
    }
}

impl HintBackground {
    fn handle_key(&mut self, key: K, value: i32, config: &Hints) -> bool {
        if key != config.keys.toggle_background || !matches!(value, 0 | 1) {
            return false;
        }
        let hidden = value == 1;
        if self.hidden == hidden {
            return false;
        }
        self.hidden = hidden;
        true
    }
}

pub struct ActiveHints {
    alphabet: LabelAlphabet,
    selection: LabelSelection,
    regions: Vec<HintRegion>,
    click: ClickKind,
    cancel: K,
    background: HintBackground,
    backend: WaylandHints,
}

impl ActiveHints {
    pub fn click_kind(&self) -> ClickKind {
        self.click
    }

    pub fn set_click_kind(&mut self, click: ClickKind) {
        self.click = click;
    }

    pub fn cancel(&mut self) {
        self.backend.destroy();
    }

    pub fn activate(config: &Hints, click: ClickKind) -> Result<Self, Box<dyn Error>> {
        let alphabet = LabelAlphabet::new(&config.label_symbols)
            .map_err(|error| format!("hints.label_symbols: {error}"))?;
        let mut backend = WaylandHints::connect()?;
        let raw_regions = backend.capture_regions(config)?;
        Self::from_capture(config, click, alphabet, backend, raw_regions)
    }

    fn from_capture(
        config: &Hints,
        click: ClickKind,
        alphabet: LabelAlphabet,
        mut backend: WaylandHints,
        raw_regions: Vec<(usize, Rect)>,
    ) -> Result<Self, Box<dyn Error>> {
        let selection = LabelSelection::new(alphabet.clone(), raw_regions.len());
        let mut regions = Vec::with_capacity(raw_regions.len());
        for (index, (output_index, rect)) in raw_regions.into_iter().enumerate() {
            regions.push(HintRegion {
                output_index,
                rect,
                label: selection.label_for_index(index).unwrap(),
            });
        }
        backend.create_overlays(config, &regions)?;
        let background = HintBackground::default();
        backend.redraw(config, &regions, &selection, background.hidden)?;
        Ok(Self {
            alphabet,
            selection,
            regions,
            click,
            cancel: config.keys.cancel,
            background,
            backend,
        })
    }

    pub fn handle_key(
        &mut self,
        key: K,
        value: i32,
        config: &Hints,
    ) -> Result<HintResult, Box<dyn Error>> {
        if key == config.keys.toggle_background {
            if self.background.handle_key(key, value, config) {
                self.backend.redraw(
                    config,
                    &self.regions,
                    &self.selection,
                    self.background.hidden,
                )?;
            }
            return Ok(HintResult::Continue);
        }
        if value != 1 {
            return Ok(HintResult::Continue);
        }
        if key == K::KEY_ESC || key == self.cancel {
            self.cancel();
            return Ok(HintResult::Cancelled);
        }
        if self.background.hidden {
            return Ok(HintResult::Continue);
        }
        if key == config.keys.debug {
            self.backend.debug_view = self.backend.debug_view.next();
            self.backend.redraw(config, &self.regions, &self.selection, false)?;
            return Ok(HintResult::Continue);
        }
        // Diagnostic rectangles have no labels. Never resolve an invisible
        // selection, and preserve its prefix until the normal view is restored.
        if self.backend.debug_view != DebugView::Normal || self.regions.is_empty() {
            return Ok(HintResult::Continue);
        }
        if key == K::KEY_BACKSPACE {
            if !self.selection.backspace() {
                self.cancel();
                return Ok(HintResult::Cancelled);
            }
            self.backend.redraw(
                config,
                &self.regions,
                &self.selection,
                self.background.hidden,
            )?;
            return Ok(HintResult::Continue);
        }
        let Some(symbol) = key_to_symbol(key) else {
            return Ok(HintResult::Continue);
        };
        let Some(index) = self.alphabet.find(symbol) else {
            return Ok(HintResult::Continue);
        };
        match self.selection.append(index) {
            AppendResult::Success => {
                if let Some(target_index) = self.selection.resolve() {
                    let region = &self.regions[target_index];
                    self.backend.click(region, self.click)?;
                    self.backend.destroy();
                    return Ok(HintResult::Clicked);
                }
                self.backend.redraw(
                    config,
                    &self.regions,
                    &self.selection,
                    self.background.hidden,
                )?;
            }
            AppendResult::Full | AppendResult::Overflow => {}
        }
        Ok(HintResult::Continue)
    }

    pub fn warning(error: &dyn Error) {
        eprintln!(
            "fievel: hint mode unavailable: {error}. \
             Continuing without hint mode; keyboard control is unaffected. \
             Requires a Wayland session with wlr-layer-shell, wlr-screencopy, \
             and wlr-virtual-pointer (e.g. Hyprland/Sway)."
        );
    }
}

struct WaylandHints {
    connection: Connection,
    queue: wayland_client::EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    sync_serial: u64,
    debug_view: DebugView,
}

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    screencopy: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    virtual_pointer: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    outputs: Vec<OutputState>,
    captures: Vec<CaptureState>,
    overlays: Vec<OverlayState>,
    completed_sync: u64,
    error: Option<String>,
}

struct OutputState {
    output: wl_output::WlOutput,
    scale: i32,
    transform: wl_output::Transform,
    pixel_width: u32,
    pixel_height: u32,
}

struct CaptureState {
    output_index: usize,
    frame: zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    format: Option<wl_shm::Format>,
    width: u32,
    height: u32,
    stride: u32,
    buffer_done: bool,
    copy_requested: bool,
    ready: bool,
    failed: bool,
    y_invert: bool,
    file: Option<File>,
    buffer: Option<wl_buffer::WlBuffer>,
}

struct OverlayState {
    output_index: usize,
    width: u32,
    height: u32,
    logical_width: u32,
    logical_height: u32,
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    file: File,
    buffer: wl_buffer::WlBuffer,
    pixels: Vec<u8>,
    configured: bool,
    closed: bool,
    trace: Option<DetectionTrace>,
}

impl Default for OutputState {
    fn default() -> Self {
        unreachable!()
    }
}

impl WaylandHints {
    fn connect() -> Result<Self, Box<dyn Error>> {
        let connection = Connection::connect_to_env()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, Box<dyn Error>> {
        let queue = connection.new_event_queue::<State>();
        let qh = queue.handle();
        connection.display().get_registry(&qh, ());
        let mut this = Self {
            connection,
            queue,
            qh,
            state: State::default(),
            sync_serial: 0,
            debug_view: DebugView::Normal,
        };
        this.sync(STARTUP_TIMEOUT)?;
        this.state
            .compositor
            .as_ref()
            .ok_or("missing wl_compositor")?;
        this.state.shm.as_ref().ok_or("missing wl_shm")?;
        this.state
            .shell
            .as_ref()
            .ok_or("compositor does not support zwlr_layer_shell_v1")?;
        this.state
            .screencopy
            .as_ref()
            .ok_or("compositor does not support zwlr_screencopy_manager_v1")?;
        let virtual_pointer = this
            .state
            .virtual_pointer
            .as_ref()
            .ok_or("compositor does not support zwlr_virtual_pointer_manager_v1")?;
        if virtual_pointer.version() < 2 {
            return Err("zwlr_virtual_pointer_manager_v1 version 2 is required".into());
        }
        if this.state.outputs.is_empty() {
            return Err("Wayland compositor reported no outputs".into());
        }
        this.state.seat.as_ref().ok_or("missing wl_seat")?;
        Ok(this)
    }

    fn capture_regions(&mut self, config: &Hints) -> Result<Vec<(usize, Rect)>, Box<dyn Error>> {
        // Configure unbuffered, invisible surfaces first. wl_output.scale is a
        // rounded integer, not the pixel/logical ratio on fractional outputs.
        self.prepare_overlays()?;
        self.state.captures.clear();
        let screencopy = self.state.screencopy.as_ref().unwrap().clone();
        for (output_index, output) in self.state.outputs.iter().enumerate() {
            let frame = screencopy.capture_output(0, &output.output, &self.qh, ());
            self.state.captures.push(CaptureState {
                output_index,
                frame,
                format: None,
                width: 0,
                height: 0,
                stride: 0,
                buffer_done: false,
                copy_requested: false,
                ready: false,
                failed: false,
                y_invert: false,
                file: None,
                buffer: None,
            });
        }
        self.pump_until(
            CAPTURE_TIMEOUT,
            "timed out waiting for screenshot capture",
            |state| {
                state
                    .captures
                    .iter()
                    .all(|capture| capture.ready || capture.failed)
            },
        )?;
        let mut regions = Vec::new();
        for capture in &mut self.state.captures {
            let Some(file) = capture.file.take() else {
                continue;
            };
            if capture.failed || !capture.ready {
                continue;
            }
            let len = (capture.stride * capture.height) as usize;
            let mut data = vec![0; len];
            let mut offset = 0;
            while offset < len {
                let read = file.read_at(&mut data[offset..], offset as u64)?;
                if read == 0 {
                    return Err("screenshot buffer ended unexpectedly".into());
                }
                offset += read;
            }
            let output = &self.state.outputs[capture.output_index];
            let rgb = normalize_capture_rgb(
                &data,
                capture.width,
                capture.height,
                capture.stride,
                capture.format.ok_or("screenshot format missing")?,
                capture.y_invert,
                output.transform,
            )?;
            let (width, height) = transformed_size(capture.width, capture.height, output.transform);
            let overlay = &mut self.state.overlays[capture.output_index];
            let detection = detect::detect_rgb_with_trace(
                &rgb,
                (width, height),
                (overlay.logical_width, overlay.logical_height),
                DetectionLimits {
                    min_width: config.min_width as f64,
                    max_width: config.max_width as f64,
                    min_height: config.min_height as f64,
                    max_height: config.max_height as f64,
                },
                config.color_links,
                config.underline_links,
            );
            overlay.trace = Some(detection.trace);
            for rect in detection.regions {
                regions.push((capture.output_index, rect));
            }
            if let Some(buffer) = capture.buffer.take() {
                buffer.destroy();
            }
        }
        // Release failed-capture buffers too. Frame events already destroy
        // their proxies; retain geometry metadata, never raw capture pixels.
        for capture in &mut self.state.captures {
            if let Some(buffer) = capture.buffer.take() {
                buffer.destroy();
            }
        }
        if self.state.overlays.iter().all(|overlay| overlay.trace.is_none()) {
            return Err("failed to capture any Wayland output".into());
        }
        Ok(regions)
    }

    fn create_overlays(
        &mut self,
        config: &Hints,
        regions: &[HintRegion],
    ) -> Result<(), Box<dyn Error>> {
        self.prepare_overlays()?;
        self.redraw(
            config,
            regions,
            &LabelSelection::new(LabelAlphabet::new(&config.label_symbols)?, regions.len()),
            false,
        )
    }

    fn prepare_overlays(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.state.overlays.is_empty() {
            return Ok(());
        }
        let compositor = self.state.compositor.as_ref().unwrap().clone();
        let shell = self.state.shell.as_ref().unwrap().clone();
        let shm = self.state.shm.as_ref().unwrap().clone();
        for (output_index, output) in self.state.outputs.iter().enumerate() {
            let width = 1;
            let height = 1;
            let logical_width = 0;
            let logical_height = 0;
            let file = File::from(memfd_create("fievel-hints", MemfdFlags::CLOEXEC)?);
            let pixels = vec![0; (width * height * 4) as usize];
            file.set_len(pixels.len() as u64)?;
            write_all_at(&file, &pixels, 0)?;
            let pool = shm.create_pool(file.as_fd(), pixels.len() as i32, &self.qh, ());
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                (width * 4) as i32,
                wl_shm::Format::Argb8888,
                &self.qh,
                (),
            );
            pool.destroy();
            let surface = compositor.create_surface(&self.qh, ());
            surface.set_buffer_scale(1);
            let region = compositor.create_region(&self.qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
            let layer = shell.get_layer_surface(
                &surface,
                Some(&output.output),
                Layer::Overlay,
                HINT_NAMESPACE.into(),
                &self.qh,
                (),
            );
            layer.set_size(0, 0);
            layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            surface.commit();
            self.state.overlays.push(OverlayState {
                output_index,
                width,
                height,
                logical_width,
                logical_height,
                surface,
                layer,
                file,
                buffer,
                pixels,
                configured: false,
                closed: false,
                trace: None,
            });
        }
        self.pump_until(
            STARTUP_TIMEOUT,
            "timed out waiting for hint overlay configuration",
            |state| {
                state
                    .overlays
                    .iter()
                    .all(|overlay| overlay.configured || overlay.closed)
            },
        )?;
        if self.state.overlays.iter().any(|overlay| overlay.closed) {
            return Err("compositor closed a hint overlay surface".into());
        }
        for overlay in &mut self.state.overlays {
            if overlay.logical_width == 0 || overlay.logical_height == 0 {
                return Err("compositor configured a zero-sized hint overlay".into());
            }
            overlay.width = overlay.logical_width;
            overlay.height = overlay.logical_height;
            overlay.pixels = vec![0; overlay.width as usize * overlay.height as usize * 4];
            overlay.file.set_len(overlay.pixels.len() as u64)?;
            let pool = shm.create_pool(
                overlay.file.as_fd(),
                overlay.pixels.len() as i32,
                &self.qh,
                (),
            );
            overlay.buffer.destroy();
            overlay.buffer = pool.create_buffer(
                0,
                overlay.width as i32,
                overlay.height as i32,
                (overlay.width * 4) as i32,
                wl_shm::Format::Argb8888,
                &self.qh,
                (),
            );
            pool.destroy();
        }
        Ok(())
    }

    fn redraw(
        &mut self,
        config: &Hints,
        regions: &[HintRegion],
        selection: &LabelSelection,
        hidden: bool,
    ) -> Result<(), Box<dyn Error>> {
        for overlay in &mut self.state.overlays {
            if overlay.closed
                || (overlay.width, overlay.height)
                    != (overlay.logical_width, overlay.logical_height)
            {
                return Err(
                    "output geometry changed during hint selection; activate hints again".into(),
                );
            }
            let mut canvas = Canvas::new(&mut overlay.pixels, overlay.width, overlay.height);
            canvas.clear(Color::rgba(0, 0, 0, 0));
            for (index, region) in regions.iter().enumerate() {
                if hidden
                    || self.debug_view != DebugView::Normal
                    || region.output_index != overlay.output_index
                    || !selection.matches_index(index)
                {
                    continue;
                }
                let rect = region.rect;
                canvas.fill_rect(
                    rect.x as i32,
                    rect.y as i32,
                    rect.width as i32,
                    rect.height as i32,
                    config.readability_color,
                );
                canvas.stroke_rect(
                    rect.x as i32,
                    rect.y as i32,
                    rect.width as i32,
                    rect.height as i32,
                    config.border_color,
                );
                let (selected, unselected) = selection.split_index(index).unwrap();
                let scale = ((rect.height as i32 - 4) / 8).clamp(1, 3);
                let (label_w, label_h) = Canvas::text_size(&region.label, scale);
                let text_x = rect.x as i32 + ((rect.width as i32 - label_w) / 2).max(1);
                let text_y = rect.y as i32 + ((rect.height as i32 - label_h) / 2).max(1);
                canvas.draw_text(
                    text_x,
                    text_y,
                    &selected,
                    scale,
                    config.label_highlight_color,
                );
                canvas.draw_text(
                    text_x + selected.chars().count() as i32 * 6 * scale,
                    text_y,
                    &unselected,
                    scale,
                    config.label_color,
                );
            }
            if !hidden {
                if self.debug_view != DebugView::Normal {
                    Self::draw_diagnostics(
                        &mut canvas, overlay.trace.as_ref(), self.debug_view,
                        overlay.width, overlay.height,
                    );
                }
                if self.debug_view != DebugView::Normal
                    || !regions.iter().any(|region| region.output_index == overlay.output_index)
                {
                    Self::draw_debug_legend(
                        &mut canvas, overlay.trace.as_ref(), self.debug_view,
                        config, overlay.width,
                    );
                }
            }
            write_all_at(&overlay.file, &overlay.pixels, 0)?;
            overlay.surface.attach(Some(&overlay.buffer), 0, 0);
            overlay.surface.damage(
                0,
                0,
                overlay.logical_width as i32,
                overlay.logical_height as i32,
            );
            overlay.surface.commit();
        }
        self.flush()?;
        Ok(())
    }

    fn outcome_color(outcome: Outcome) -> Color {
        match outcome {
            Outcome::Accepted => Color::rgba(80, 255, 120, 255),
            Outcome::InsufficientEdges => Color::rgba(100, 190, 255, 255),
            Outcome::TooSmall => Color::rgba(255, 215, 70, 255),
            Outcome::TooLarge => Color::rgba(255, 90, 90, 255),
            Outcome::NestedOrDuplicate => Color::rgba(220, 130, 255, 255),
            Outcome::NotTextLike => Color::rgba(255, 150, 70, 255),
        }
    }

    fn draw_diagnostics(
        canvas: &mut Canvas<'_>,
        trace: Option<&DetectionTrace>,
        view: DebugView,
        width: u32,
        height: u32,
    ) {
        canvas.clear(Color::rgba(12, 16, 20, 240));
        let Some(trace) = trace else { return };
        let edge_color = if view == DebugView::Edges {
            Color::rgba(210, 235, 255, 255)
        } else {
            Color::rgba(90, 100, 110, 255)
        };
        for y in 0..trace.height.min(height) {
            for x in 0..trace.width.min(width) {
                if trace.edges[(y * trace.width + x) as usize] != 0 {
                    canvas.fill_rect(x as i32, y as i32, 1, 1, edge_color);
                }
                if trace.color_strokes.get((y * trace.width + x) as usize) == Some(&1) {
                    canvas.fill_rect(x as i32, y as i32, 1, 1, Color::rgba(255, 180, 80, 255));
                }
            }
        }
        if view == DebugView::Components {
            for component in &trace.components {
                let rect = component.bounds;
                canvas.stroke_rect(
                    rect.x as i32, rect.y as i32, rect.width as i32, rect.height as i32,
                    Self::outcome_color(component.outcome),
                );
                if component.source == detect::Source::Underline {
                    canvas.fill_rect(rect.x as i32, rect.y as i32, 3, 3,
                        Color::rgba(80, 235, 235, 255));
                }
            }
        }
        for line in trace.components.iter().filter_map(|c| c.stroke_bounds) {
            canvas.stroke_rect(line.x as i32, line.y as i32, line.width as i32,
                line.height as i32, Color::rgba(80, 235, 235, 255));
        }
    }

    fn key_caption(key: K) -> String {
        format!("{key:?}").trim_start_matches("KEY_").replace('_', " ")
    }

    fn draw_debug_legend(
        canvas: &mut Canvas<'_>,
        trace: Option<&DetectionTrace>,
        view: DebugView,
        config: &Hints,
        width: u32,
    ) {
        let white = Color::rgba(255, 255, 255, 255);
        let title = match view {
            DebugView::Normal => "NO TARGETS",
            DebugView::Edges => "EDGES COLOR AND UNDERLINES",
            DebugView::Components => "GROUPED COMPONENT BOUNDS",
        };
        let mut lines = vec![(title.to_owned(), white)];
        if let Some(trace) = trace {
            let edge_pixels = trace.components.iter()
                .filter(|c| c.source == detect::Source::Grayscale).map(|c| c.edge_pixels).sum::<usize>();
            lines.push((format!("COMPONENTS {}  EDGE PIXELS {}",
                trace.components.len(), edge_pixels), white));
            let icons: Vec<_> = trace.components.iter()
                .filter(|c| c.source == detect::Source::Icon).collect();
            lines.push((format!("ICON CONTOURS {}  ACCEPTED {}",
                icons.len(), icons.iter().filter(|c| c.outcome == Outcome::Accepted).count()), white));
            let color_components = trace.components.iter()
                .filter(|c| c.source == detect::Source::Color).count();
            let color_pixels = trace.components.iter()
                .filter(|c| c.source == detect::Source::Color).map(|c| c.edge_pixels).sum::<usize>();
            lines.push((format!("COLOR COMPONENTS {color_components}  STROKES {color_pixels}"),
                Color::rgba(255, 180, 80, 255)));
            let underlines: Vec<_> = trace.components.iter()
                .filter(|c| c.source == detect::Source::Underline).collect();
            lines.push((format!("UNDERLINE {}  ACCEPTED {}  REJECTED {}",
                underlines.len(),
                underlines.iter().filter(|c| c.outcome == Outcome::Accepted).count(),
                underlines.iter().filter(|c| c.outcome != Outcome::Accepted).count()),
                Color::rgba(80, 235, 235, 255)));
            lines.push(("CYAN UNDERLINE EXTENT AND SOURCE MARK".to_owned(),
                Color::rgba(80, 235, 235, 255)));
            for (outcome, label) in [
                (Outcome::Accepted, "ACCEPTED"),
                (Outcome::TooSmall, "TOO SMALL"),
                (Outcome::TooLarge, "TOO LARGE"),
                (Outcome::InsufficientEdges, "INSUFFICIENT SAMPLES"),
                (Outcome::NotTextLike, "NOT TEXT LIKE"),
                (Outcome::NestedOrDuplicate, "NESTED OR DUPLICATE"),
            ] {
                let count = trace.components.iter().filter(|c| c.outcome == outcome).count();
                lines.push((format!("{label} {count}"), Self::outcome_color(outcome)));
            }
        } else {
            lines.push(("CAPTURE UNAVAILABLE".to_owned(), white));
        }
        lines.push((format!("{} NEXT VIEW  ESC EXIT", Self::key_caption(config.keys.debug)), white));
        lines.push((format!("HOLD {} TO PEEK", Self::key_caption(config.keys.toggle_background)), white));
        if view != DebugView::Normal {
            lines.push(("SELECTION PAUSED  SAME SNAPSHOT".to_owned(), white));
        }
        let longest = lines.iter().map(|(text, _)| text.len()).max().unwrap_or(1) as i32;
        let scale = if longest * 12 + 16 <= width as i32 { 2 } else { 1 };
        let columns = ((width as i32 - 16) / (6 * scale)).max(1) as usize;
        let mut y = 8;
        for (text, color) in lines {
            // Key names and captions are ASCII; wrap even custom long key names.
            for chunk in text.as_bytes().chunks(columns) {
                let text = std::str::from_utf8(chunk).unwrap();
                canvas.fill_rect(4, y - 3, (text.len() as i32 * 6 + 1) * scale + 8,
                    10 * scale, Color::rgba(12, 16, 20, 255));
                canvas.draw_text(8, y, text, scale, color);
                y += 10 * scale;
            }
        }
    }

    fn click(&mut self, region: &HintRegion, click: ClickKind) -> Result<(), Box<dyn Error>> {
        let output = self.state.outputs[region.output_index].output.clone();
        let overlay = &self.state.overlays[region.output_index];
        let (x, y, extent_width, extent_height) =
            click_coordinates(region.rect, overlay.width, overlay.height);

        // Unmap before moving so the compositor can restore the underlying
        // surface's focus. A flush alone does not acknowledge request handling.
        self.destroy_overlays();
        self.sync(INPUT_TIMEOUT)?;
        let pointer = self
            .state
            .virtual_pointer
            .as_ref()
            .unwrap()
            .create_virtual_pointer_with_output(
                self.state.seat.as_ref(),
                Some(&output),
                &self.qh,
                (),
            );
        pointer.motion_absolute(time_ms(), x, y, extent_width, extent_height);
        pointer.frame();
        let result = self
            .sync(INPUT_TIMEOUT)
            .and_then(|()| self.click_button(&pointer, click));
        pointer.destroy();
        // Keep the connection/device alive until release has been processed,
        // and acknowledge device destruction even when an earlier step failed.
        let cleanup = self.sync(INPUT_TIMEOUT);
        result.and(cleanup)
    }

    fn click_button(
        &mut self,
        pointer: &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
        click: ClickKind,
    ) -> Result<(), Box<dyn Error>> {
        let button = match click {
            ClickKind::Left => K::BTN_LEFT.0 as u32,
            ClickKind::Right => K::BTN_RIGHT.0 as u32,
        };
        pointer.button(time_ms(), button, wl_pointer::ButtonState::Pressed);
        pointer.frame();
        let pressed = self.sync(INPUT_TIMEOUT);
        // Never return after sending a press without also queuing its release.
        // On a broken connection this is best effort; device teardown follows.
        pointer.button(time_ms(), button, wl_pointer::ButtonState::Released);
        pointer.frame();
        let released = self.sync(INPUT_TIMEOUT);
        pressed.and(released)
    }

    fn destroy_overlays(&mut self) {
        for overlay in self.state.overlays.drain(..) {
            overlay.layer.destroy();
            overlay.surface.destroy();
            overlay.buffer.destroy();
        }
    }

    fn destroy(&mut self) {
        self.destroy_overlays();
        let _ = self.flush();
    }

    fn flush(&self) -> Result<(), Box<dyn Error>> {
        let deadline = std::time::Instant::now() + INPUT_TIMEOUT;
        while !self.try_flush()? {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err("timed out flushing Wayland requests".into());
            }
            let mut fds = [PollFd::new(&self.connection, PollFlags::OUT)];
            match poll(
                &mut fds,
                Some(&Timespec {
                    tv_sec: remaining.as_secs() as i64,
                    tv_nsec: remaining.subsec_nanos() as i64,
                }),
            ) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn try_flush(&self) -> Result<bool, Box<dyn Error>> {
        match self.connection.flush() {
            Ok(()) => Ok(true),
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == io::ErrorKind::WouldBlock =>
            {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn sync(&mut self, timeout: Duration) -> Result<(), Box<dyn Error>> {
        self.sync_serial += 1;
        let serial = self.sync_serial;
        self.connection.display().sync(&self.qh, serial);
        self.pump_until(
            timeout,
            "timed out synchronizing Wayland requests",
            |state| state.completed_sync == serial,
        )
    }

    fn pump_until<F>(
        &mut self,
        timeout: Duration,
        message: &str,
        mut ready: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(&State) -> bool,
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            self.queue.dispatch_pending(&mut self.state)?;
            if let Some(error) = self.state.error.take() {
                return Err(error.into());
            }
            if ready(&self.state) {
                return Ok(());
            }
            let writable = !self.try_flush()?;
            let Some(read) = self.queue.prepare_read() else {
                continue;
            };
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                drop(read);
                return Err(message.into());
            }
            let mut fds = [PollFd::new(
                &self.connection,
                PollFlags::IN
                    | if writable {
                        PollFlags::OUT
                    } else {
                        PollFlags::empty()
                    },
            )];
            match poll(
                &mut fds,
                Some(&Timespec {
                    tv_sec: remaining.as_secs() as i64,
                    tv_nsec: remaining.subsec_nanos() as i64,
                }),
            ) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            if fds[0]
                .revents()
                .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
            {
                match read.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(error))
                        if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
            } else {
                drop(read);
            }
        }
    }
}

impl Drop for WaylandHints {
    fn drop(&mut self) {
        self.destroy();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, 1, qh, ()));
                }
                "zwlr_layer_shell_v1" => state.shell = Some(registry.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.virtual_pointer = Some(registry.bind(name, version.min(2), qh, ()))
                }
                "wl_output" => {
                    let output = registry.bind(name, version.min(4), qh, ());
                    state.outputs.push(OutputState {
                        output,
                        scale: 1,
                        transform: wl_output::Transform::Normal,
                        pixel_width: 1,
                        pixel_height: 1,
                    });
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, u64> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        serial: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.completed_sync = *serial;
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(target) = state
            .outputs
            .iter_mut()
            .find(|candidate| &candidate.output == output)
        else {
            return;
        };
        match event {
            wl_output::Event::Geometry { transform, .. } => {
                if let WEnum::Value(transform) = transform {
                    target.transform = transform;
                }
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                if matches!(flags, WEnum::Value(bits) if bits.contains(wl_output::Mode::Current)) {
                    target.pixel_width = width as u32;
                    target.pixel_height = height as u32;
                }
            }
            wl_output::Event::Scale { factor } => target.scale = factor.max(1),
            _ => {}
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        frame: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(index) = state
            .captures
            .iter()
            .position(|capture| &capture.frame == frame)
        else {
            return;
        };
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let WEnum::Value(format) = format {
                    state.captures[index].format = Some(format);
                    state.captures[index].width = width;
                    state.captures[index].height = height;
                    state.captures[index].stride = stride;
                    if frame.version() < 3 {
                        if let Err(error) = request_copy(state, index, qh) {
                            state.error = Some(format!("screenshot allocation failed: {error}"));
                            state.captures[index].failed = true;
                            frame.destroy();
                        }
                    }
                } else {
                    state.captures[index].failed = true;
                    frame.destroy();
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                state.captures[index].buffer_done = true;
                if let Err(error) = request_copy(state, index, qh) {
                    state.error = Some(format!("screenshot allocation failed: {error}"));
                    state.captures[index].failed = true;
                    frame.destroy();
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                state.captures[index].y_invert = matches!(flags, WEnum::Value(bits) if bits.contains(zwlr_screencopy_frame_v1::Flags::YInvert));
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.captures[index].ready = true;
                frame.destroy();
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.captures[index].failed = true;
                frame.destroy();
            }
            _ => {}
        }
    }
}

fn request_copy(
    state: &mut State,
    index: usize,
    qh: &QueueHandle<State>,
) -> Result<(), Box<dyn Error>> {
    let shm = state.shm.as_ref().ok_or("missing wl_shm")?.clone();
    let capture = &mut state.captures[index];
    if capture.copy_requested || capture.width == 0 || capture.height == 0 {
        return Ok(());
    }
    if capture.frame.version() >= 3 && !capture.buffer_done {
        return Ok(());
    }
    let len = (capture.stride * capture.height) as usize;
    let file = File::from(memfd_create("fievel-screencopy", MemfdFlags::CLOEXEC)?);
    file.set_len(len as u64)?;
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        capture.width as i32,
        capture.height as i32,
        capture.stride as i32,
        capture.format.ok_or("missing screenshot format")?,
        qh,
        (),
    );
    pool.destroy();
    capture.frame.copy(&buffer);
    capture.copy_requested = true;
    capture.file = Some(file);
    capture.buffer = Some(buffer);
    Ok(())
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(overlay) = state
            .overlays
            .iter_mut()
            .find(|overlay| &overlay.layer == layer)
        else {
            return;
        };
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                overlay.logical_width = width;
                overlay.logical_height = height;
                overlay.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => overlay.closed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_region::WlRegion);
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(State: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);

fn key_to_symbol(key: K) -> Option<char> {
    Some(match key {
        K::KEY_A => 'a',
        K::KEY_B => 'b',
        K::KEY_C => 'c',
        K::KEY_D => 'd',
        K::KEY_E => 'e',
        K::KEY_F => 'f',
        K::KEY_G => 'g',
        K::KEY_H => 'h',
        K::KEY_I => 'i',
        K::KEY_J => 'j',
        K::KEY_K => 'k',
        K::KEY_L => 'l',
        K::KEY_M => 'm',
        K::KEY_N => 'n',
        K::KEY_O => 'o',
        K::KEY_P => 'p',
        K::KEY_Q => 'q',
        K::KEY_R => 'r',
        K::KEY_S => 's',
        K::KEY_T => 't',
        K::KEY_U => 'u',
        K::KEY_V => 'v',
        K::KEY_W => 'w',
        K::KEY_X => 'x',
        K::KEY_Y => 'y',
        K::KEY_Z => 'z',
        _ => return None,
    })
}

fn write_all_at(file: &File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = file.write_at(bytes, offset)?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write to shared buffer",
            ));
        }
        bytes = &bytes[written..];
        offset += written as u64;
    }
    Ok(())
}

fn normalize_capture_rgb(
    data: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
    y_invert: bool,
    transform: wl_output::Transform,
) -> Result<Vec<detect::Rgb>, Box<dyn Error>> {
    if width == 0
        || height == 0
        || (stride as usize) < width as usize * 4
        || data.len() < stride as usize * height as usize
    {
        return Err("invalid screenshot buffer dimensions or stride".into());
    }
    let mut rgb = vec![[0; 3]; (width * height) as usize];
    for y in 0..height as usize {
        let source_y = if y_invert { height as usize - 1 - y } else { y };
        let row =
            &data[source_y * stride as usize..source_y * stride as usize + width as usize * 4];
        for x in 0..width as usize {
            let pixel = &row[x * 4..x * 4 + 4];
            let (r, g, b) = match format {
                wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => {
                    (pixel[2], pixel[1], pixel[0])
                }
                wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => {
                    (pixel[0], pixel[1], pixel[2])
                }
                _ => return Err(format!("unsupported screenshot format {format:?}").into()),
            };
            rgb[y * width as usize + x] = [r, g, b];
        }
    }
    Ok(transform_pixels(&rgb, width, height, transform))
}

#[cfg(test)]
fn normalize_capture(
    data: &[u8], width: u32, height: u32, stride: u32, format: wl_shm::Format,
    y_invert: bool, transform: wl_output::Transform,
) -> Result<Vec<u8>, Box<dyn Error>> {
    normalize_capture_rgb(data, width, height, stride, format, y_invert, transform)
        .map(|rgb| detect::luma(&rgb))
}

fn transformed_size(width: u32, height: u32, transform: wl_output::Transform) -> (u32, u32) {
    match transform {
        wl_output::Transform::Normal
        | wl_output::Transform::Flipped
        | wl_output::Transform::Flipped180
        | wl_output::Transform::_180 => (width, height),
        wl_output::Transform::_90
        | wl_output::Transform::_270
        | wl_output::Transform::Flipped90
        | wl_output::Transform::Flipped270 => (height, width),
        _ => (width, height),
    }
}

fn transform_pixels<T: Copy + Default>(
    pixels: &[T],
    width: u32,
    height: u32,
    transform: wl_output::Transform,
) -> Vec<T> {
    let (out_width, out_height) = transformed_size(width, height, transform);
    let mut out = vec![T::default(); (out_width * out_height) as usize];
    for y in 0..out_height {
        for x in 0..out_width {
            let (sx, sy) = match transform {
                wl_output::Transform::Normal => (x, y),
                wl_output::Transform::_90 => (y, height - 1 - x),
                wl_output::Transform::_180 => (width - 1 - x, height - 1 - y),
                wl_output::Transform::_270 => (width - 1 - y, x),
                wl_output::Transform::Flipped => (width - 1 - x, y),
                wl_output::Transform::Flipped90 => (y, x),
                wl_output::Transform::Flipped180 => (x, height - 1 - y),
                wl_output::Transform::Flipped270 => (width - 1 - y, height - 1 - x),
                _ => (x, y),
            };
            out[(y * out_width + x) as usize] = pixels[(sy * width + sx) as usize];
        }
    }
    out
}

fn time_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u32
}

fn click_coordinates(rect: Rect, width: u32, height: u32) -> (u32, u32, u32, u32) {
    (
        (rect.x + rect.width / 2).min(width.saturating_sub(1)),
        (rect.y + rect.height / 2).min(height.saturating_sub(1)),
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_is_held_and_only_changes_on_transitions() {
        let config = Hints::default();
        let mut background = HintBackground::default();
        assert!(!background.hidden);
        assert!(!background.handle_key(K::KEY_LEFTCTRL, 0, &config));
        assert!(!background.handle_key(K::KEY_LEFTCTRL, 2, &config));
        assert!(!background.handle_key(K::KEY_RIGHTCTRL, 1, &config));
        assert!(background.handle_key(K::KEY_LEFTCTRL, 1, &config));
        assert!(background.hidden);
        for value in [1, 2, 2, -1] {
            assert!(!background.handle_key(K::KEY_LEFTCTRL, value, &config));
            assert!(background.hidden);
        }
        assert!(!background.handle_key(K::KEY_RIGHTCTRL, 0, &config));
        assert!(background.handle_key(K::KEY_LEFTCTRL, 0, &config));
        assert!(!background.hidden);
        assert!(!background.handle_key(K::KEY_LEFTCTRL, 0, &config));
        assert!(!background.handle_key(K::KEY_LEFTCTRL, 2, &config));
        assert!(background.handle_key(K::KEY_LEFTCTRL, 1, &config));
        assert!(background.hidden);
        assert!(!HintBackground::default().hidden);
    }

    #[test]
    fn background_uses_custom_key_and_fill_beneath_border_and_text() {
        let mut config = Hints::default();
        config.keys.toggle_background = K::KEY_RIGHTCTRL;
        config.readability_color = Color::rgba(80, 80, 80, 255);
        let mut background = HintBackground::default();
        assert!(!background.handle_key(K::KEY_LEFTCTRL, 1, &config));
        assert!(background.handle_key(K::KEY_RIGHTCTRL, 1, &config));
        assert!(background.hidden);
        assert!(background.handle_key(K::KEY_RIGHTCTRL, 0, &config));
        let mut pixels = vec![0; 20 * 20 * 4];
        let mut canvas = Canvas::new(&mut pixels, 20, 20);
        canvas.fill_rect(1, 1, 18, 18, config.readability_color);
        canvas.stroke_rect(1, 1, 18, 18, Color::rgba(0, 255, 0, 255));
        canvas.draw_text(5, 5, "a", 1, Color::rgba(255, 255, 255, 255));
        assert_eq!(
            &pixels[(2 * 20 + 2) * 4..(2 * 20 + 3) * 4],
            &[80, 80, 80, 255],
        );
        assert_eq!(&pixels[(20 + 1) * 4..(20 + 2) * 4], &[0, 255, 0, 255],);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel == [255; 4]));
        assert_eq!(&pixels[..4], &[0; 4]);
        assert!(!background.hidden);
    }

    #[test]
    fn key_mapping_accepts_only_a_to_z() {
        assert_eq!(key_to_symbol(K::KEY_A), Some('a'));
        assert_eq!(key_to_symbol(K::KEY_Z), Some('z'));
        assert_eq!(key_to_symbol(K::KEY_SPACE), None);
    }

    #[test]
    fn transform_and_size_follow_output_orientation() {
        let luma = vec![1, 2, 3, 4, 5, 6];
        use wl_output::Transform::*;
        for (transform, size, expected) in [
            (Normal, (3, 2), vec![1, 2, 3, 4, 5, 6]),
            (_90, (2, 3), vec![4, 1, 5, 2, 6, 3]),
            (_180, (3, 2), vec![6, 5, 4, 3, 2, 1]),
            (_270, (2, 3), vec![3, 6, 2, 5, 1, 4]),
            (Flipped, (3, 2), vec![3, 2, 1, 6, 5, 4]),
            (Flipped90, (2, 3), vec![1, 4, 2, 5, 3, 6]),
            (Flipped180, (3, 2), vec![4, 5, 6, 1, 2, 3]),
            (Flipped270, (2, 3), vec![6, 3, 5, 2, 4, 1]),
        ] {
            assert_eq!(transformed_size(3, 2, transform), size);
            assert_eq!(transform_pixels(&luma, 3, 2, transform), expected);
            let rgb: Vec<_> = luma.iter().map(|&v| [v, v + 10, v + 20]).collect();
            let expected_rgb: Vec<_> = expected.iter().map(|&v| [v, v + 10, v + 20]).collect();
            assert_eq!(transform_pixels(&rgb, 3, 2, transform), expected_rgb);
        }
    }

    #[test]
    fn capture_render_and_click_share_fractional_output_local_coordinates() {
        for (pixels_w, pixels_h, logical_w, logical_h) in [
            (1920, 1080, 1536, 864),
            (1080, 1920, 864, 1536),
            (1920, 1080, 960, 540),
            (1920, 1080, 1920, 1080),
        ] {
            // Output-local coordinates must not include the desktop output's
            // position or use wl_output's rounded integer scale.
            let pixel_rect = Rect {
                x: pixels_w * 3 / 4,
                y: pixels_h * 3 / 4,
                width: pixels_w / 8,
                height: pixels_h / 8,
            };
            let logical = detect::map_rect(pixel_rect, pixels_w, pixels_h, logical_w, logical_h);
            let (x, y, extent_w, extent_h) = click_coordinates(logical, logical_w, logical_h);
            assert!((x as f64 / extent_w as f64 - 0.8125).abs() < 0.002);
            assert!((y as f64 / extent_h as f64 - 0.8125).abs() < 0.002);
            let mut pixels = vec![0; logical_w as usize * logical_h as usize * 4];
            Canvas::new(&mut pixels, logical_w, logical_h).fill_rect(
                logical.x as i32,
                logical.y as i32,
                logical.width as i32,
                logical.height as i32,
                Color::rgba(0, 255, 0, 255),
            );
            let index = (y * logical_w + x) as usize * 4;
            assert_eq!(&pixels[index..index + 4], &[0, 255, 0, 255]);
        }
    }

    #[test]
    fn normalize_capture_reads_argb_pixels() {
        let data = vec![0x00, 0x00, 0xff, 0xff, 0x00, 0xff, 0x00, 0xff];
        let luma = normalize_capture(
            &data,
            2,
            1,
            8,
            wl_shm::Format::Argb8888,
            false,
            wl_output::Transform::Normal,
        )
        .unwrap();
        assert_eq!(luma.len(), 2);
        assert!(luma[1] > luma[0]);
    }

    #[test]
    fn capture_normalization_honors_padding_inversion_and_format() {
        let data = [
            255, 0, 0, 255, 11, 22, 33, 44, 0, 255, 0, 255, 55, 66, 77, 88,
        ];
        for format in [wl_shm::Format::Abgr8888, wl_shm::Format::Xbgr8888] {
            assert_eq!(
                normalize_capture_rgb(&data, 1, 2, 8, format, true, wl_output::Transform::_90).unwrap(),
                [[255, 0, 0], [0, 255, 0]],
            );
            assert_eq!(
                normalize_capture(&data, 1, 2, 8, format, true, wl_output::Transform::_90,)
                    .unwrap(),
                vec![76, 149]
            );
        }
        for format in [wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888] {
            assert_eq!(
                normalize_capture_rgb(&data, 1, 2, 8, format, true, wl_output::Transform::_90).unwrap(),
                [[0, 0, 255], [0, 255, 0]],
            );
        }
        assert!(normalize_capture(
            &data[..3],
            1,
            1,
            4,
            wl_shm::Format::Argb8888,
            false,
            wl_output::Transform::Normal
        )
        .is_err());
        assert!(normalize_capture(
            &data,
            2,
            2,
            4,
            wl_shm::Format::Argb8888,
            false,
            wl_output::Transform::Normal
        )
        .is_err());
        assert!(normalize_capture(
            &data,
            1,
            1,
            4,
            wl_shm::Format::Rgb565,
            false,
            wl_output::Transform::Normal
        )
        .is_err());
    }

    #[test]
    #[ignore = "requires Wayland screencopy; measures targets in memory, no overlay, input grab or click"]
    fn wayland_detection_geometry_smoke() {
        let mut backend = WaylandHints::connect().unwrap();
        let started = std::time::Instant::now();
        let regions = backend.capture_regions(&Hints::default()).unwrap();
        for (i, output) in backend.state.outputs.iter().enumerate() {
            let overlay = &backend.state.overlays[i];
            let capture = &backend.state.captures[i];
            assert!(
                capture.ready && !capture.failed,
                "output {i} capture failed"
            );
            let oriented = transformed_size(capture.width, capture.height, output.transform);
            let count = regions.iter().filter(|(index, _)| *index == i).count();
            let trace = overlay.trace.as_ref().expect("successful capture has diagnostics");
            assert_eq!((trace.width, trace.height), (overlay.width, overlay.height));
            assert_eq!(trace.components.iter().filter(|c| c.outcome == Outcome::Accepted).count(), count);
            assert_eq!(trace.edges.len(), (overlay.width * overlay.height) as usize);
            assert!(capture.file.is_none() && capture.buffer.is_none(), "raw capture resources retained");
            eprintln!(
                "output {i}: mode={}x{}, capture={}x{}, oriented={}x{}, logical={}x{}, wl_scale={}, actual_scale={:.3}x{:.3}, transform={:?}, targets={count}",
                output.pixel_width, output.pixel_height, capture.width, capture.height,
                oriented.0, oriented.1, overlay.width, overlay.height, output.scale,
                oriented.0 as f64 / overlay.width as f64,
                oriented.1 as f64 / overlay.height as f64, output.transform,
            );
            for (_, rect) in regions.iter().filter(|(index, _)| *index == i) {
                assert!(rect.width > 0 && rect.height > 0);
                assert!(rect.x + rect.width <= overlay.width);
                assert!(rect.y + rect.height <= overlay.height);
                let (x, y, w, h) = click_coordinates(*rect, overlay.width, overlay.height);
                assert!(x < w && y < h);
            }
        }
        eprintln!("capture + detection: {:?}", started.elapsed());
    }

    #[test]
    #[ignore = "requires a Wayland compositor with wlr protocols"]
    fn wayland_hints_lifecycle() {
        let config = Hints::default();
        let mut hints = ActiveHints::activate(&config, ClickKind::Left).unwrap();
        let _ = hints.handle_key(K::KEY_ESC, 1, &config).unwrap();
    }

    #[test]
    #[ignore = "requires a Wayland compositor; briefly displays a synthetic hint without grabbing input"]
    fn wayland_background_hold_repaints_visible_pixels() {
        fn screenshot(backend: &mut WaylandHints) -> Vec<u8> {
            let frame = backend.state.screencopy.as_ref().unwrap().capture_output(
                0,
                &backend.state.outputs[0].output,
                &backend.qh,
                (),
            );
            backend.state.captures.push(CaptureState {
                output_index: 0,
                frame,
                format: None,
                width: 0,
                height: 0,
                stride: 0,
                buffer_done: false,
                copy_requested: false,
                ready: false,
                failed: false,
                y_invert: false,
                file: None,
                buffer: None,
            });
            backend
                .pump_until(CAPTURE_TIMEOUT, "test screenshot timed out", |state| {
                    state
                        .captures
                        .last()
                        .is_some_and(|capture| capture.ready || capture.failed)
                })
                .unwrap();
            let capture = backend.state.captures.pop().unwrap();
            assert!(capture.ready && !capture.failed);
            let mut data = vec![0; (capture.stride * capture.height) as usize];
            capture
                .file
                .as_ref()
                .unwrap()
                .read_exact_at(&mut data, 0)
                .unwrap();
            let transform = backend.state.outputs[0].transform;
            let luma = normalize_capture(
                &data,
                capture.width,
                capture.height,
                capture.stride,
                capture.format.unwrap(),
                capture.y_invert,
                transform,
            )
            .unwrap();
            capture.buffer.unwrap().destroy();
            let (width, height) = transformed_size(capture.width, capture.height, transform);
            let overlay = &backend.state.overlays[0];
            detect::resize_luma(&luma, width, height, overlay.width, overlay.height)
        }

        let config = Hints {
            fill_color: Color::rgba(0, 255, 0, 255),
            readability_color: Color::rgba(64, 64, 64, 255),
            ..Hints::default()
        };
        let mut backend = WaylandHints::connect().unwrap();
        backend.capture_regions(&config).unwrap();
        let unobscured = screenshot(&mut backend);
        let alphabet = LabelAlphabet::new(&config.label_symbols).unwrap();
        let selection = LabelSelection::new(alphabet.clone(), 1);
        let regions = vec![HintRegion {
            output_index: 0,
            rect: Rect {
                x: 40,
                y: 40,
                width: 200,
                height: 60,
            },
            label: selection.label_for_index(0).unwrap(),
        }];
        backend.create_overlays(&config, &regions).unwrap();
        let mut hints = ActiveHints {
            alphabet,
            selection,
            regions,
            click: ClickKind::Left,
            cancel: config.keys.cancel,
            background: HintBackground::default(),
            backend,
        };
        std::thread::sleep(Duration::from_millis(250));
        let initial = screenshot(&mut hints.backend);
        hints.handle_key(K::KEY_LEFTCTRL, 1, &config).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let held = screenshot(&mut hints.backend);
        hints.handle_key(K::KEY_LEFTCTRL, 0, &config).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let restored = screenshot(&mut hints.backend);
        let width = hints.backend.state.overlays[0].width as usize;
        // Probe known logical positions, not merely any repainted screen pixels.
        for (x, y) in [(45, 45), (235, 45), (45, 95), (235, 95)] {
            let index = y * width + x;
            assert!((i32::from(initial[index]) - 64).abs() <= 3);
            assert!((i32::from(held[index]) - i32::from(unobscured[index])).abs() <= 3);
            assert!((i32::from(restored[index]) - 64).abs() <= 3);
        }
    }
}

#[cfg(test)]
mod click_protocol_tests {
    use super::*;
    use std::{
        collections::HashMap,
        io::{Read, Write},
        os::unix::net::UnixStream,
        thread,
        time::Instant,
    };

    #[derive(Debug, PartialEq, Eq)]
    enum Request {
        Sync(usize),
        LayerDestroyed,
        SurfaceDestroyed,
        PointerCreated,
        Motion([u32; 4]),
        Button(u32, u32),
        Frame,
        PointerDestroyed,
    }

    fn send_event(socket: &mut UnixStream, object: u32, opcode: u16, payload: &[u8]) {
        socket.write_all(&object.to_ne_bytes()).unwrap();
        socket
            .write_all(&(((payload.len() as u32 + 8) << 16) | opcode as u32).to_ne_bytes())
            .unwrap();
        socket.write_all(payload).unwrap();
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect()
    }

    // This private socket speaks the real client wire protocol, including
    // registry binding and sync callbacks. It never connects to the desktop.
    // It verifies requests, not compositor-specific wl_pointer event delivery.
    fn serve(mut socket: UnixStream, withheld_sync: Option<usize>) -> Vec<Request> {
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let globals = [
            ("wl_compositor", 4),
            ("wl_shm", 1),
            ("zwlr_layer_shell_v1", 1),
            ("zwlr_screencopy_manager_v1", 3),
            ("zwlr_virtual_pointer_manager_v1", 2),
            ("wl_output", 4),
            ("wl_seat", 1),
        ];
        let mut objects = HashMap::from([(1, "wl_display")]);
        let mut layers = HashMap::new();
        let mut requests = Vec::new();
        let mut sync_count = 0;
        let mut held_button = None;
        loop {
            let mut header = [0; 8];
            match socket.read_exact(&mut header) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => panic!("mock Wayland request failed: {error}"),
            }
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let size_opcode = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let opcode = size_opcode as u16;
            let mut payload = vec![0; (size_opcode >> 16) as usize - 8];
            socket.read_exact(&mut payload).unwrap();
            let args: Vec<u32> = payload
                .chunks_exact(4)
                .map(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()))
                .collect();
            match (objects[&object], opcode) {
                ("wl_display", 1) => {
                    let registry = args[0];
                    objects.insert(registry, "wl_registry");
                    for (index, (interface, version)) in globals.iter().enumerate() {
                        let mut global = words(&[index as u32 + 1, interface.len() as u32 + 1]);
                        global.extend_from_slice(interface.as_bytes());
                        global.push(0);
                        while global.len() % 4 != 0 {
                            global.push(0);
                        }
                        global.extend_from_slice(&u32::to_ne_bytes(*version));
                        send_event(&mut socket, registry, 0, &global);
                    }
                }
                ("wl_display", 0) => {
                    sync_count += 1;
                    requests.push(Request::Sync(sync_count));
                    if withheld_sync != Some(sync_count) {
                        send_event(&mut socket, args[0], 0, &words(&[sync_count as u32]));
                        send_event(&mut socket, 1, 1, &words(&[args[0]]));
                    }
                }
                ("wl_registry", 0) => {
                    let (interface, version) = globals[args[0] as usize - 1];
                    assert!(args[args.len() - 2] <= version);
                    objects.insert(*args.last().unwrap(), interface);
                }
                ("wl_compositor", 0) => {
                    objects.insert(args[0], "wl_surface");
                }
                ("wl_compositor", 1) => {
                    objects.insert(args[0], "wl_region");
                }
                ("wl_shm", 0) => {
                    objects.insert(args[0], "wl_shm_pool");
                }
                ("wl_shm_pool", 0) => {
                    objects.insert(args[0], "wl_buffer");
                }
                ("zwlr_layer_shell_v1", 0) => {
                    objects.insert(args[0], "zwlr_layer_surface_v1");
                    layers.insert(args[1], args[0]);
                }
                ("wl_surface", 6) => {
                    if let Some(layer) = layers.get(&object) {
                        send_event(&mut socket, *layer, 0, &words(&[1, 640, 480]));
                    }
                }
                ("zwlr_layer_surface_v1", 7) => requests.push(Request::LayerDestroyed),
                ("wl_surface", 0) => requests.push(Request::SurfaceDestroyed),
                ("zwlr_virtual_pointer_manager_v1", 2) => {
                    assert_eq!(objects.get(&args[0]), Some(&"wl_seat"));
                    assert_eq!(objects.get(&args[1]), Some(&"wl_output"));
                    objects.insert(args[2], "zwlr_virtual_pointer_v1");
                    requests.push(Request::PointerCreated);
                }
                ("zwlr_virtual_pointer_v1", 1) => {
                    requests.push(Request::Motion(args[1..5].try_into().unwrap()));
                }
                ("zwlr_virtual_pointer_v1", 2) => {
                    let (button, state) = (args[1], args[2]);
                    if state == 1 {
                        assert!(held_button.replace(button).is_none());
                    } else {
                        assert_eq!(held_button.take(), Some(button));
                    }
                    requests.push(Request::Button(button, state));
                }
                ("zwlr_virtual_pointer_v1", 4) => requests.push(Request::Frame),
                ("zwlr_virtual_pointer_v1", 8) => {
                    assert_eq!(held_button, None, "device destroyed with a held button");
                    requests.push(Request::PointerDestroyed);
                }
                _ => {}
            }
        }
        assert_eq!(held_button, None, "connection closed with a held button");
        requests
    }

    fn run_click(click: ClickKind, withheld_sync: Option<usize>) -> (bool, Vec<Request>, Duration) {
        let (client, server) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || serve(server, withheld_sync));
        let connection = Connection::from_socket(client).unwrap();
        let mut backend = WaylandHints::from_connection(connection).unwrap();
        backend.prepare_overlays().unwrap();
        let started = Instant::now();
        let result = backend.click(
            &HintRegion {
                output_index: 0,
                rect: Rect {
                    x: 100,
                    y: 100,
                    width: 40,
                    height: 20,
                },
                label: "a".into(),
            },
            click,
        );
        let elapsed = started.elapsed();
        let success = result.is_ok();
        drop(backend);
        (success, server.join().unwrap(), elapsed)
    }

    fn successful_sequence(button: K) -> Vec<Request> {
        use Request::*;
        vec![
            Sync(1),
            LayerDestroyed,
            SurfaceDestroyed,
            Sync(2),
            PointerCreated,
            Motion([120, 110, 640, 480]),
            Frame,
            Sync(3),
            Button(button.0 as u32, 1),
            Frame,
            Sync(4),
            Button(button.0 as u32, 0),
            Frame,
            Sync(5),
            PointerDestroyed,
            Sync(6),
        ]
    }

    #[test]
    fn left_and_right_clicks_have_separate_acknowledged_frames() {
        for (click, button) in [
            (ClickKind::Left, K::BTN_LEFT),
            (ClickKind::Right, K::BTN_RIGHT),
        ] {
            let (success, requests, _) = run_click(click, None);
            assert!(success);
            assert_eq!(requests, successful_sequence(button));
        }
    }

    #[test]
    fn press_timeout_still_releases_and_destroys_both_buttons() {
        for (click, button) in [
            (ClickKind::Left, K::BTN_LEFT),
            (ClickKind::Right, K::BTN_RIGHT),
        ] {
            let (success, requests, elapsed) = run_click(click, Some(4));
            assert!(!success);
            assert_eq!(requests, successful_sequence(button));
            assert!(elapsed < INPUT_TIMEOUT * 3);
        }
    }

    #[test]
    fn focus_timeout_destroys_pointer_without_pressing() {
        use Request::*;
        let (success, requests, elapsed) = run_click(ClickKind::Left, Some(3));
        assert!(!success);
        assert_eq!(
            requests,
            vec![
                Sync(1),
                LayerDestroyed,
                SurfaceDestroyed,
                Sync(2),
                PointerCreated,
                Motion([120, 110, 640, 480]),
                Frame,
                Sync(3),
                PointerDestroyed,
                Sync(4),
            ]
        );
        assert!(elapsed < INPUT_TIMEOUT * 3);
    }

    #[test]
    fn overlay_removal_timeout_never_creates_pointer() {
        use Request::*;
        let (success, requests, elapsed) = run_click(ClickKind::Right, Some(2));
        assert!(!success);
        assert_eq!(
            requests,
            vec![Sync(1), LayerDestroyed, SurfaceDestroyed, Sync(2)]
        );
        assert!(elapsed < INPUT_TIMEOUT * 3);
    }

    #[test]
    fn debug_cycles_zero_targets_and_selection_without_clicking_and_peek_restores() {
        for (count, color_links, underline_links) in [
            (0, false, false), (27, false, false), (27, true, false),
            (27, false, true), (27, true, true),
        ] {
            let (client, server) = UnixStream::pair().unwrap();
            let server = thread::spawn(move || serve(server, None));
            let mut backend =
                WaylandHints::from_connection(Connection::from_socket(client).unwrap()).unwrap();
            backend.prepare_overlays().unwrap();
            let config = crate::config::Config::parse(include_str!("../fievel.config")).unwrap();
            let mut luma = vec![255; 640 * 480];
            for index in 0..count {
                let x = 10 + index % 9 * 60;
                let y = 280 + index / 9 * 50;
                for yy in y..y + 20 {
                    for xx in x..x + 30 {
                        luma[yy * 640 + xx] = 0;
                    }
                }
            }
            let mut detection = detect::detect_with_trace(&luma, 640, 480, 1.0, DetectionLimits::default());
            assert_eq!(detection.regions.len(), count);
            if color_links || underline_links {
                let mut pixels: Vec<_> = luma.iter().flat_map(|&v| [v, v, v, 255]).collect();
                let mut canvas = Canvas::new(&mut pixels, 640, 480);
                canvas.draw_text(10, 240, "TEXT", 2, Color::rgba(90, 90, 90, 255));
                let link_color = if color_links { Color::rgba(30, 90, 210, 255) }
                    else { Color::rgba(90, 90, 90, 255) };
                canvas.draw_text(58, 240, "LINK", 2, link_color);
                canvas.draw_text(106, 240, "TEXT", 2, Color::rgba(90, 90, 90, 255));
                if underline_links {
                    canvas.fill_rect(58, 256, 46, 1, link_color);
                }
                let rgb: Vec<_> = pixels.chunks_exact(4).map(|p| [p[2], p[1], p[0]]).collect();
                detection = detect::detect_rgb_with_trace(&rgb, (640, 480), (640, 480),
                    DetectionLimits::default(), color_links, underline_links);
                assert_eq!(detection.regions.len(), count + 2);
                assert!(detection.trace.components.iter()
                    .any(|c| c.source == if color_links { detect::Source::Color }
                        else { detect::Source::Underline } && c.outcome == Outcome::Accepted));
                if underline_links {
                    assert!(detection.trace.components.iter().any(|c|
                        c.source == detect::Source::Underline && c.stroke_bounds.is_some()));
                }
            }
            backend.state.overlays[0].trace = Some(detection.trace);
            let mut hints = ActiveHints::from_capture(
                &config.hints, ClickKind::Left,
                LabelAlphabet::new(&config.hints.label_symbols).unwrap(),
                backend, detection.regions.into_iter().map(|rect| (0, rect)).collect(),
            ).unwrap();
            if count > 0 {
                hints.handle_key(K::KEY_A, 1, &config.hints).unwrap();
            }
            let selection = hints.selection.clone();
            let normal = hints.backend.state.overlays[0].pixels.clone();
            assert!(normal.iter().any(|byte| *byte != 0), "zero targets must still be visible");
            for view in [DebugView::Edges, DebugView::Components] {
                hints.handle_key(config.hints.keys.debug, 1, &config.hints).unwrap();
                hints.handle_key(config.hints.keys.debug, 2, &config.hints).unwrap();
                assert_eq!(hints.backend.debug_view, view);
                let visible = hints.backend.state.overlays[0].pixels.clone();
                if underline_links {
                    assert!(visible.chunks_exact(4).any(|p| p == [235, 235, 80, 255]));
                }
                for key in [K::KEY_A, K::KEY_Z, K::KEY_BACKSPACE] {
                    assert!(matches!(hints.handle_key(key, 1, &config.hints).unwrap(), HintResult::Continue));
                }
                assert_eq!(hints.selection, selection);
                let mut input = crate::hint_input::HintInput::new(&config).unwrap();
                input.begin([]);
                let mut mode = Some(hints);
                for (key, value) in [(K::KEY_S, 1), (K::KEY_D, 1)] {
                    crate::handle_hint_input(input.key(key, value), &mut mode, &config);
                }
                assert!(mode.as_ref().unwrap().backend.state.overlays[0].pixels.iter().all(|byte| *byte == 0));
                crate::handle_hint_input(
                    vec![crate::hint_input::HintInputEvent::Key(config.hints.keys.debug, 1)],
                    &mut mode, &config,
                );
                assert_eq!(mode.as_ref().unwrap().backend.debug_view, view);
                crate::handle_hint_input(input.key(K::KEY_S, 0), &mut mode, &config);
                crate::handle_hint_input(input.key(K::KEY_D, 0), &mut mode, &config);
                hints = mode.unwrap();
                assert_eq!(hints.backend.debug_view, view);
                assert_eq!(hints.backend.state.overlays[0].pixels, visible);
                assert_eq!(hints.selection, selection);
            }
            hints.handle_key(config.hints.keys.debug, 1, &config.hints).unwrap();
            assert_eq!(hints.backend.debug_view, DebugView::Normal);
            assert_eq!(hints.backend.state.overlays[0].pixels, normal);
            if count == 0 {
                for key in [K::KEY_A, K::KEY_BACKSPACE] {
                    assert!(matches!(hints.handle_key(key, 1, &config.hints).unwrap(), HintResult::Continue));
                }
            }
            hints.handle_key(config.hints.keys.debug, 1, &config.hints).unwrap();
            let mut mode = Some(hints);
            crate::handle_hint_input(vec![crate::hint_input::HintInputEvent::Shortcut(ClickKind::Right)],
                &mut mode, &config);
            assert_eq!(mode.as_ref().unwrap().click_kind(), ClickKind::Right);
            assert_eq!(mode.as_ref().unwrap().backend.debug_view, DebugView::Edges);
            mode.as_mut().unwrap().backend.sync(INPUT_TIMEOUT).unwrap();
            if count == 0 {
                crate::handle_hint_input(vec![crate::hint_input::HintInputEvent::Key(K::KEY_ESC, 1)],
                    &mut mode, &config);
            } else {
                crate::handle_hint_input(vec![crate::hint_input::HintInputEvent::Shortcut(ClickKind::Right)],
                    &mut mode, &config);
            }
            assert!(mode.is_none());
            let requests = server.join().unwrap();
            assert!(!requests.iter().any(|request| matches!(request, Request::PointerCreated)));
        }
    }

    #[test]
    fn peek_hold_hides_entire_overlay_and_restores_prefix_without_selecting() {
        let (client, server) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || serve(server, None));
        let mut backend =
            WaylandHints::from_connection(Connection::from_socket(client).unwrap()).unwrap();
        backend.prepare_overlays().unwrap();
        let mut config = Hints::default();
        config.keys.toggle_background = K::KEY_F6;
        config.fill_color = Color::rgba(0, 255, 0, 255);
        config.readability_color = Color::rgba(64, 64, 64, 255);
        let alphabet = LabelAlphabet::new(&config.label_symbols).unwrap();
        let selection = LabelSelection::new(alphabet.clone(), 27);
        let mut hints = ActiveHints {
            alphabet,
            regions: vec![HintRegion {
                output_index: 0,
                rect: Rect {
                    x: 10,
                    y: 10,
                    width: 60,
                    height: 30,
                },
                label: selection.label_for_index(0).unwrap(),
            }],
            selection,
            click: ClickKind::Left,
            cancel: K::KEY_ESC,
            background: HintBackground::default(),
            backend,
        };
        hints.handle_key(K::KEY_A, 1, &config).unwrap();
        let prefix = hints.selection.split_index(0).unwrap();
        let initial_pixels = hints.backend.state.overlays[0].pixels.clone();
        assert!(initial_pixels.iter().any(|byte| *byte != 0));
        let probe = (12 * 640 + 12) * 4;
        for (value, visible, redraw) in [
            (1, true, true),
            (1, true, false),
            (2, true, false),
            (0, false, true),
            (0, false, false),
            (2, false, false),
        ] {
            hints.backend.state.overlays[0].pixels[probe..probe + 4].copy_from_slice(&[1, 2, 3, 4]);
            assert!(matches!(
                hints.handle_key(K::KEY_F6, value, &config).unwrap(),
                HintResult::Continue
            ));
            assert_eq!(hints.background.hidden, visible);
            let expected = if !redraw {
                [1, 2, 3, 4]
            } else if visible {
                [0; 4]
            } else {
                [64, 64, 64, 255]
            };
            assert_eq!(
                hints.backend.state.overlays[0].pixels[probe..probe + 4],
                expected
            );
            if visible && redraw {
                assert!(hints.backend.state.overlays[0].pixels.iter().all(|byte| *byte == 0));
                for key in [K::KEY_A, K::KEY_BACKSPACE] {
                    assert!(matches!(
                        hints.handle_key(key, 1, &config).unwrap(),
                        HintResult::Continue
                    ));
                }
                assert!(hints.backend.state.overlays[0].pixels.iter().all(|byte| *byte == 0));
            }
            assert_eq!(hints.selection.split_index(0).unwrap(), prefix);
            if !visible && redraw {
                assert_eq!(hints.backend.state.overlays[0].pixels, initial_pixels);
            }
        }
        hints.cancel();
        hints.backend.sync(INPUT_TIMEOUT).unwrap();
        drop(hints);
        let requests = server.join().unwrap();
        assert!(!requests
            .iter()
            .any(|request| matches!(request, Request::PointerCreated)));
    }
}
