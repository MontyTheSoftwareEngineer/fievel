use crate::{
    config::Hints,
    detect::{self, DetectionLimits, Outcome, Rect, Source},
    font::{Canvas, Color},
    grid::{GridQuadrant, GridRect, GridSelection},
    input::KeyCode as K,
    label::{AppendResult, LabelAlphabet, LabelSelection},
    windows_ui::{HintColors, Overlay, OverlayHint},
};
use std::{
    error::Error,
    io,
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    GetMonitorInfoW, MonitorFromPoint, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows_sys::Win32::{
    Foundation::POINT,
    UI::WindowsAndMessaging::GetCursorPos,
};

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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
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

#[derive(Clone)]
struct HintRegion {
    rect: Rect,
}

#[derive(Clone, Copy)]
struct DebugRegion {
    rect: Rect,
    outcome: Outcome,
    source: Source,
    edge_pixels: usize,
    stroke_bounds: Option<Rect>,
}

pub struct ActiveHints {
    alphabet: LabelAlphabet,
    selection: LabelSelection,
    regions: Vec<HintRegion>,
    debug_regions: Vec<DebugRegion>,
    edge_regions: Vec<Rect>,
    click: ClickKind,
    cancel: K,
    debug_key: K,
    background_key: K,
    background_hidden: bool,
    debug_view: DebugView,
    origin: (i32, i32),
    grid: Option<GridSelection>,
}

impl ActiveHints {
    pub fn click_kind(&self) -> ClickKind {
        self.click
    }

    pub fn set_click_kind(&mut self, click: ClickKind) {
        self.click = click;
    }

    pub fn is_grid_mode(&self) -> bool {
        self.grid.is_some()
    }

    pub fn cancel(&mut self, overlay: &mut Overlay) -> io::Result<()> {
        overlay.clear_hints()
    }

    pub fn activate(
        config: &Hints,
        click: ClickKind,
        overlay: &mut Overlay,
    ) -> Result<Self, Box<dyn Error>> {
        let alphabet = LabelAlphabet::new(&config.label_symbols)
            .map_err(|error| format!("hints.label_symbols: {error}"))?;
        let origin = overlay.origin();
        let (regions, debug_regions, edge_regions) = capture_regions(config, overlay)?;
        let selection = LabelSelection::new(alphabet.clone(), regions.len());
        let hints = Self {
            alphabet,
            selection,
            regions,
            debug_regions,
            edge_regions,
            click,
            cancel: config.keys.cancel,
            debug_key: config.keys.debug,
            background_key: config.keys.toggle_background,
            background_hidden: false,
            debug_view: DebugView::Normal,
            origin,
            grid: None,
        };
        hints.redraw(config, overlay)?;
        Ok(hints)
    }

    pub fn activate_grid(
        config: &Hints,
        click: ClickKind,
        overlay: &mut Overlay,
    ) -> Result<Self, Box<dyn Error>> {
        let alphabet = LabelAlphabet::new(&config.label_symbols)
            .map_err(|error| format!("hints.label_symbols: {error}"))?;
        let origin = overlay.origin();
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let mut info: MONITORINFO = unsafe { zeroed() };
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut grid = GridSelection::new(GridRect {
            x: f64::from(info.rcMonitor.left - origin.0),
            y: f64::from(info.rcMonitor.top - origin.1),
            width: f64::from(info.rcMonitor.right - info.rcMonitor.left),
            height: f64::from(info.rcMonitor.bottom - info.rcMonitor.top),
        });
        grid.set_cursor(
            f64::from(point.x - origin.0),
            f64::from(point.y - origin.1),
        );
        let hints = Self {
            alphabet: alphabet.clone(),
            selection: LabelSelection::new(alphabet, 0),
            regions: Vec::new(),
            debug_regions: Vec::new(),
            edge_regions: Vec::new(),
            click,
            cancel: config.keys.cancel,
            debug_key: config.keys.debug,
            background_key: config.keys.toggle_background,
            background_hidden: false,
            debug_view: DebugView::Normal,
            origin,
            grid: Some(grid),
        };
        hints.redraw(config, overlay)?;
        Ok(hints)
    }

    pub fn handle_key(
        &mut self,
        key: K,
        value: i32,
        config: &Hints,
        overlay: &mut Overlay,
    ) -> Result<HintResult, Box<dyn Error>> {
        if key == self.background_key && matches!(value, 0 | 1) {
            self.background_hidden = value == 1;
            self.redraw(config, overlay)?;
            return Ok(HintResult::Continue);
        }
        if value != 1 {
            return Ok(HintResult::Continue);
        }
        if key == K::KEY_ESC || key == self.cancel {
            self.cancel(overlay)?;
            return Ok(HintResult::Cancelled);
        }
        if key == self.debug_key && self.grid.is_none() {
            self.debug_view = self.debug_view.next();
            self.redraw(config, overlay)?;
            return Ok(HintResult::Continue);
        }
        if self.background_hidden && self.grid.is_some() {
            return Ok(HintResult::Continue);
        }
        if let Some(grid) = &mut self.grid {
            if key == K::KEY_BACKSPACE {
                if !grid.backspace() {
                    self.cancel(overlay)?;
                    return Ok(HintResult::Cancelled);
                }
                let (x, y) = grid.cursor();
                Overlay::move_cursor_to(
                    self.origin.0 + x.round() as i32,
                    self.origin.1 + y.round() as i32,
                )?;
                self.redraw(config, overlay)?;
                return Ok(HintResult::Continue);
            }
            if key == config.keys.grid_left_click || key == config.keys.grid_right_click {
                let (x, y) = grid.cursor();
                overlay.click_at(
                    self.origin.0 + x.round() as i32,
                    self.origin.1 + y.round() as i32,
                    key == config.keys.grid_right_click,
                )?;
                self.cancel(overlay)?;
                return Ok(HintResult::Clicked);
            }
            let Some(quadrant) = GridQuadrant::from_key(key, &config.keys) else {
                return Ok(HintResult::Continue);
            };
            if grid.select(quadrant) {
                let (x, y) = grid.cursor();
                Overlay::move_cursor_to(
                    self.origin.0 + x.round() as i32,
                    self.origin.1 + y.round() as i32,
                )?;
                self.redraw(config, overlay)?;
            }
            return Ok(HintResult::Continue);
        }
        if self.background_hidden || self.debug_view != DebugView::Normal || self.regions.is_empty()
        {
            return Ok(HintResult::Continue);
        }
        if key == K::KEY_BACKSPACE {
            if !self.selection.backspace() {
                self.cancel(overlay)?;
                return Ok(HintResult::Cancelled);
            }
            self.redraw(config, overlay)?;
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
                    let region = &self.regions[target_index].rect;
                    let x = self.origin.0 + region.x as i32 + (region.width / 2) as i32;
                    let y = self.origin.1 + region.y as i32 + (region.height / 2) as i32;
                    overlay.click_at(x, y, self.click == ClickKind::Right)?;
                    self.cancel(overlay)?;
                    return Ok(HintResult::Clicked);
                }
                self.redraw(config, overlay)?;
            }
            AppendResult::Full | AppendResult::Overflow => {}
        }
        Ok(HintResult::Continue)
    }

    fn redraw(&self, config: &Hints, overlay: &mut Overlay) -> io::Result<()> {
        let colors = HintColors {
            border: config.border_color,
            fill: config.fill_color,
            readability: config.readability_color,
            text: config.label_color,
            highlight: config.label_highlight_color,
        };
        if let Some(grid) = &self.grid {
            let mut hints = Vec::with_capacity(8);
            for (quadrant, rect) in grid.cells() {
                let x = self.origin.0 + rect.x.round() as i32;
                let y = self.origin.1 + rect.y.round() as i32;
                let width = rect.width.round() as i32;
                let height = rect.height.round() as i32;
                let label = quadrant.label(&config.keys);
                let label_width = (Canvas::text_size(&label, 2).0 + 12).max(24);
                hints.push(OverlayHint {
                    x,
                    y,
                    width,
                    height,
                    label: String::new(),
                    prefix_length: 0,
                    border: Some(config.grid_border_color(quadrant)),
                    fill: Some(config.grid_fill_color(quadrant)),
                    debug_dot: false,
                });
                hints.push(OverlayHint {
                    x: x + width / 2 - label_width / 2,
                    y: y + height / 2 - 10,
                    width: label_width,
                    height: 20,
                    label,
                    prefix_length: 0,
                    border: None,
                    fill: Some(config.readability_color),
                    debug_dot: false,
                });
            }
            return overlay.set_hints(hints, self.background_hidden, colors);
        }
        let hints = match self.debug_view {
            DebugView::Normal => self
                .regions
                .iter()
                .enumerate()
                .filter_map(|(index, region)| {
                    if !self.selection.matches_index(index) {
                        return None;
                    }
                    let (prefix, suffix) = self.selection.split_index(index)?;
                    let prefix_length = prefix.chars().count();
                    Some(OverlayHint {
                        x: self.origin.0 + region.rect.x as i32,
                        y: self.origin.1 + region.rect.y as i32,
                        width: region.rect.width as i32,
                        height: region.rect.height as i32,
                        label: prefix + &suffix,
                        prefix_length,
                        border: None,
                        fill: None,
                        debug_dot: false,
                    })
                })
                .collect(),
            DebugView::Edges => self
                .edge_regions
                .iter()
                .map(|rect| OverlayHint {
                    x: self.origin.0 + rect.x as i32,
                    y: self.origin.1 + rect.y as i32,
                    width: rect.width as i32,
                    height: rect.height as i32,
                    label: String::new(),
                    prefix_length: 0,
                    border: Some(Color::rgba(0, 255, 0, 255)),
                    fill: Some(Color::rgba(0, 255, 0, 255)),
                    debug_dot: true,
                })
                .collect(),
            DebugView::Components => self
                .debug_regions
                .iter()
                .map(|region| {
                    let rect = region.stroke_bounds.unwrap_or(region.rect);
                    let border = outcome_color(region.outcome, region.source);
                    let alpha = if region.edge_pixels > 16 { 0x80 } else { 0x30 };
                    OverlayHint {
                        x: self.origin.0 + rect.x as i32,
                        y: self.origin.1 + rect.y as i32,
                        width: rect.width as i32,
                        height: rect.height as i32,
                        label: String::new(),
                        prefix_length: 0,
                        border: Some(border),
                        fill: Some(Color::rgba(border.r, border.g, border.b, alpha)),
                        debug_dot: false,
                    }
                })
                .collect(),
        };
        overlay.set_hints(hints, self.background_hidden, colors)
    }

    pub fn warning(error: &dyn Error) {
        eprintln!(
            "fievel: hint mode unavailable: {error}. Continuing without hint mode; \
             keyboard control is unaffected. Protected desktop content may not be capturable."
        );
    }
}

fn capture_regions(
    config: &Hints,
    overlay: &mut Overlay,
) -> Result<(Vec<HintRegion>, Vec<DebugRegion>, Vec<Rect>), Box<dyn Error>> {
    overlay.hide_for_capture();
    let captured = capture_desktop(overlay);
    let restored = overlay.show_after_capture();
    let (rgb, width, height) = captured?;
    restored?;
    let detection = detect::detect_rgb_with_trace(
        &rgb,
        (width, height),
        (width, height),
        DetectionLimits {
            min_width: config.min_width as f64,
            max_width: config.max_width as f64,
            min_height: config.min_height as f64,
            max_height: config.max_height as f64,
        },
        true,
        true,
    );
    let edge_stride = detection
        .trace
        .edges
        .iter()
        .filter(|edge| **edge != 0)
        .count()
        .div_ceil(1200)
        .max(1);
    let mut edge_regions = Vec::new();
    let mut sampled_edges = 0;
    for (index, edge) in detection.trace.edges.iter().enumerate() {
        if *edge == 0 || index % edge_stride != 0 || sampled_edges >= 1200 {
            continue;
        }
        sampled_edges += 1;
        edge_regions.push(Rect {
            x: index as u32 % detection.trace.width,
            y: index as u32 / detection.trace.width,
            width: 1,
            height: 1,
        });
    }
    let debug_regions = detection
        .trace
        .components
        .into_iter()
        .take(600)
        .map(|component| DebugRegion {
            rect: component.bounds,
            outcome: component.outcome,
            source: component.source,
            edge_pixels: component.edge_pixels,
            stroke_bounds: component.stroke_bounds,
        })
        .collect();
    let regions = detection
        .regions
        .into_iter()
        .map(|rect| HintRegion { rect })
        .collect();
    Ok((regions, debug_regions, edge_regions))
}

struct DesktopCapture {
    screen_dc: HDC,
    memory_dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
}

impl Drop for DesktopCapture {
    fn drop(&mut self) {
        unsafe {
            if !self.memory_dc.is_null() {
                SelectObject(self.memory_dc, self.previous_bitmap);
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                DeleteDC(self.memory_dc);
            }
            if !self.screen_dc.is_null() {
                ReleaseDC(null_mut(), self.screen_dc);
            }
        }
    }
}

fn capture_desktop(overlay: &Overlay) -> Result<(Vec<detect::Rgb>, u32, u32), Box<dyn Error>> {
    let (width, height) = overlay.size();
    let (origin_x, origin_y) = overlay.origin();
    let width_i32 = i32::try_from(width)?;
    let height_i32 = i32::try_from(height)?;
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or("desktop capture dimensions are too large")?;
    let screen_dc = unsafe { GetDC(null_mut()) };
    if screen_dc.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if memory_dc.is_null() {
        let error = io::Error::last_os_error();
        unsafe { ReleaseDC(null_mut(), screen_dc) };
        return Err(error.into());
    }
    let mut capture = DesktopCapture {
        screen_dc,
        memory_dc,
        bitmap: null_mut(),
        previous_bitmap: null_mut(),
    };
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width_i32,
        biHeight: -height_i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { zeroed() }
    };
    let mut bits = null_mut();
    capture.bitmap =
        unsafe { CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if capture.bitmap.is_null() || bits.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    capture.previous_bitmap = unsafe { SelectObject(memory_dc, capture.bitmap) };
    if capture.previous_bitmap.is_null() {
        return Err(io::Error::last_os_error().into());
    }
    if unsafe {
        BitBlt(
            memory_dc, 0, 0, width_i32, height_i32, screen_dc, origin_x, origin_y, SRCCOPY,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let pixels = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), pixel_count * 4) };
    let rgb = pixels
        .chunks_exact(4)
        .map(|pixel| [pixel[2], pixel[1], pixel[0]])
        .collect();
    Ok((rgb, width, height))
}

fn outcome_color(outcome: Outcome, source: Source) -> Color {
    let rgba: u32 = if outcome != Outcome::Accepted {
        match outcome {
            Outcome::TooSmall | Outcome::TooLarge => 0xff3030ff,
            Outcome::InsufficientEdges => 0xffa000ff,
            Outcome::NestedOrDuplicate => 0x909090ff,
            Outcome::NotTextLike => 0xc000ffff,
            Outcome::Accepted => unreachable!(),
        }
    } else {
        match source {
            Source::Grayscale => 0x00ff00ff,
            Source::Icon => 0x00cfffff,
            Source::Color => 0xff00d0ff,
            Source::Underline => 0xffd000ff,
        }
    };
    Color::rgba(
        (rgba >> 24) as u8,
        (rgba >> 16) as u8,
        (rgba >> 8) as u8,
        rgba as u8,
    )
}

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
