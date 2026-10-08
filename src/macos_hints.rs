use crate::{
    config::Hints,
    detect::{self, DetectionLimits, Outcome, Rect, Source},
    label::{AppendResult, LabelAlphabet, LabelSelection},
};
use evdev::KeyCode as K;
use std::{error::Error, ffi::CString, os::raw::c_char, ptr};

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

#[repr(C)]
struct NativeCapture {
    display_id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    pixel_width: u32,
    pixel_height: u32,
    stride: u32,
    rgba: *const u8,
}

#[repr(C)]
struct NativeHint {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    label: *const c_char,
    selected: bool,
    prefix_length: u32,
    custom_colors: bool,
    debug_dot: bool,
    border_color: u32,
    fill_color: u32,
}

#[derive(Clone, Copy)]
struct HintRegion {
    rect: Rect,
    display_x: f64,
    display_y: f64,
}

#[derive(Clone, Copy)]
struct DebugRegion {
    rect: Rect,
    display_x: f64,
    display_y: f64,
    outcome: Outcome,
    source: Source,
    edge_pixels: usize,
    stroke_bounds: Option<Rect>,
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

pub struct ActiveHints {
    alphabet: LabelAlphabet,
    selection: LabelSelection,
    regions: Vec<HintRegion>,
    debug_regions: Vec<DebugRegion>,
    edge_regions: Vec<DebugRegion>,
    click: ClickKind,
    background_hidden: bool,
    cancel: K,
    debug_key: K,
    debug_view: DebugView,
}

unsafe extern "C" {
    fn fievel_request_screen_capture_permission() -> bool;
    fn fievel_capture_screens(captures: *mut *mut NativeCapture, count: *mut usize) -> bool;
    fn fievel_release_captures(captures: *mut NativeCapture, count: usize);
    fn fievel_main_display_height() -> f64;
    fn fievel_ui_set_hints(
        hints: *const NativeHint,
        count: usize,
        hidden: bool,
        border: u32,
        fill: u32,
        readability: u32,
        text: u32,
        highlight: u32,
    );
    fn fievel_ui_clear_hints();
    fn fievel_click_at(x: f64, y: f64, right: bool) -> bool;
}

struct CaptureGuard {
    captures: *mut NativeCapture,
    count: usize,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        unsafe { fievel_release_captures(self.captures, self.count) };
    }
}

impl ActiveHints {
    pub fn click_kind(&self) -> ClickKind {
        self.click
    }

    pub fn set_click_kind(&mut self, click: ClickKind) {
        self.click = click;
    }

    pub fn cancel(&mut self) {
        unsafe { fievel_ui_clear_hints() };
    }

    pub fn activate(config: &Hints, click: ClickKind) -> Result<Self, Box<dyn Error>> {
        let alphabet = LabelAlphabet::new(&config.label_symbols)
            .map_err(|error| format!("hints.label_symbols: {error}"))?;
        let (regions, debug_regions, edge_regions) = capture_regions(config)?;
        let selection = LabelSelection::new(alphabet.clone(), regions.len());
        let hints = Self {
            alphabet,
            selection,
            regions,
            debug_regions,
            edge_regions,
            click,
            background_hidden: false,
            cancel: config.keys.cancel,
            debug_key: config.keys.debug,
            debug_view: DebugView::Normal,
        };
        hints.redraw(config);
        Ok(hints)
    }

    pub fn handle_key(
        &mut self,
        key: K,
        value: i32,
        config: &Hints,
    ) -> Result<HintResult, Box<dyn Error>> {
        if key == config.keys.toggle_background && matches!(value, 0 | 1) {
            self.background_hidden = value == 1;
            self.redraw(config);
            return Ok(HintResult::Continue);
        }
        if value != 1 {
            return Ok(HintResult::Continue);
        }
        if key == K::KEY_ESC || key == self.cancel {
            self.cancel();
            return Ok(HintResult::Cancelled);
        }
        if key == self.debug_key {
            self.debug_view = self.debug_view.next();
            self.redraw(config);
            return Ok(HintResult::Continue);
        }
        if self.background_hidden || self.debug_view != DebugView::Normal || self.regions.is_empty()
        {
            return Ok(HintResult::Continue);
        }
        if key == K::KEY_BACKSPACE {
            if !self.selection.backspace() {
                self.cancel();
                return Ok(HintResult::Cancelled);
            }
            self.redraw(config);
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
                    let region = self.regions[target_index];
                    let x = region.display_x
                        + f64::from(region.rect.x)
                        + f64::from(region.rect.width) / 2.0;
                    let y = region.display_y
                        + f64::from(region.rect.y)
                        + f64::from(region.rect.height) / 2.0;
                    if !unsafe { fievel_click_at(x, y, self.click == ClickKind::Right) } {
                        return Err("failed to synthesize the macOS hint click".into());
                    }
                    self.cancel();
                    return Ok(HintResult::Clicked);
                }
                self.redraw(config);
            }
            AppendResult::Full | AppendResult::Overflow => {}
        }
        Ok(HintResult::Continue)
    }

    fn redraw(&self, config: &Hints) {
        let main_height = unsafe { fievel_main_display_height() };
        let (labels, native) = match self.debug_view {
            DebugView::Normal => {
                let mut labels = Vec::new();
                let mut native = Vec::new();
                for (index, region) in self.regions.iter().enumerate() {
                    if !self.selection.matches_index(index) {
                        continue;
                    }
                    let (prefix, suffix) = self.selection.split_index(index).unwrap();
                    let prefix_length = prefix.chars().count() as u32;
                    let label =
                        CString::new(prefix + &suffix).expect("hint label contains no NUL bytes");
                    native.push(NativeHint {
                        x: region.display_x + f64::from(region.rect.x),
                        y: main_height
                            - (region.display_y
                                + f64::from(region.rect.y)
                                + f64::from(region.rect.height)),
                        width: f64::from(region.rect.width),
                        height: f64::from(region.rect.height),
                        label: label.as_ptr(),
                        selected: true,
                        prefix_length,
                        custom_colors: false,
                        debug_dot: false,
                        border_color: 0,
                        fill_color: 0,
                    });
                    labels.push(label);
                }
                (labels, native)
            }
            DebugView::Edges => (
                Vec::new(),
                self.edge_regions
                    .iter()
                    .map(|region| debug_hint(*region, main_height, true))
                    .collect(),
            ),
            DebugView::Components => (
                Vec::new(),
                self.debug_regions
                    .iter()
                    .map(|region| debug_hint(*region, main_height, false))
                    .collect(),
            ),
        };
        let _labels = labels;
        unsafe {
            fievel_ui_set_hints(
                native.as_ptr(),
                native.len(),
                self.background_hidden,
                color(config.border_color),
                color(config.fill_color),
                color(config.readability_color),
                color(config.label_color),
                color(config.label_highlight_color),
            );
        }
    }

    pub fn warning(error: &dyn Error) {
        eprintln!(
            "fievel: hint mode unavailable: {error}. Continuing without hint mode; \
             keyboard control is unaffected. Grant Screen Recording permission in \
             System Settings > Privacy & Security."
        );
    }
}

impl Drop for ActiveHints {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn capture_regions(
    config: &Hints,
) -> Result<(Vec<HintRegion>, Vec<DebugRegion>, Vec<DebugRegion>), Box<dyn Error>> {
    if !unsafe { fievel_request_screen_capture_permission() } {
        return Err(
            "macOS Screen Recording permission was not granted; enable it for the terminal or app, then restart Fievel"
                .into(),
        );
    }
    let mut captures = ptr::null_mut();
    let mut count = 0;
    if !unsafe { fievel_capture_screens(&mut captures, &mut count) } || captures.is_null() {
        return Err("macOS could not capture any active display".into());
    }
    let guard = CaptureGuard { captures, count };
    let result =
        unsafe {
            let captures = std::slice::from_raw_parts(guard.captures, guard.count);
            let mut regions = Vec::new();
            let mut debug_regions = Vec::new();
            let mut edge_regions = Vec::new();
            for capture in captures {
                if capture.rgba.is_null()
                    || capture.pixel_width == 0
                    || capture.pixel_height == 0
                    || capture.width <= 0.0
                    || capture.height <= 0.0
                {
                    continue;
                }
                let pixel_count = (capture.pixel_width as usize)
                    .checked_mul(capture.pixel_height as usize)
                    .ok_or("display dimensions are too large")?;
                let data_len = (capture.stride as usize)
                    .checked_mul(capture.pixel_height as usize)
                    .ok_or("display buffer dimensions are too large")?;
                let data = std::slice::from_raw_parts(capture.rgba, data_len);
                let mut rgb = Vec::with_capacity(pixel_count);
                for pixel in data.chunks_exact(4).take(pixel_count) {
                    rgb.push([pixel[0], pixel[1], pixel[2]]);
                }
                let logical_width = capture.width.round().max(1.0) as u32;
                let logical_height = capture.height.round().max(1.0) as u32;
                let detection = detect::detect_rgb_with_trace(
                    &rgb,
                    (capture.pixel_width, capture.pixel_height),
                    (logical_width, logical_height),
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
                let mut sampled_edges = 0;
                for (index, edge) in detection.trace.edges.iter().enumerate() {
                    if *edge == 0 || index % edge_stride != 0 || sampled_edges >= 1200 {
                        continue;
                    }
                    sampled_edges += 1;
                    edge_regions.push(DebugRegion {
                        rect: Rect {
                            x: index as u32 % detection.trace.width,
                            y: index as u32 / detection.trace.width,
                            width: 1,
                            height: 1,
                        },
                        display_x: capture.x,
                        display_y: capture.y,
                        outcome: Outcome::Accepted,
                        source: Source::Grayscale,
                        edge_pixels: 1,
                        stroke_bounds: None,
                    });
                }
                debug_regions.extend(detection.trace.components.iter().take(600).map(
                    |component| DebugRegion {
                        rect: component.bounds,
                        display_x: capture.x,
                        display_y: capture.y,
                        outcome: component.outcome,
                        source: component.source,
                        edge_pixels: component.edge_pixels,
                        stroke_bounds: component.stroke_bounds,
                    },
                ));
                for rect in detection.regions {
                    regions.push(HintRegion {
                        rect,
                        display_x: capture.x,
                        display_y: capture.y,
                    });
                }
            }
            (regions, debug_regions, edge_regions)
        };
    drop(guard);
    Ok(result)
}

fn debug_hint(region: DebugRegion, main_height: f64, dot: bool) -> NativeHint {
    let border_color = if region.outcome != Outcome::Accepted {
        match region.outcome {
            Outcome::TooSmall | Outcome::TooLarge => 0xff3030ff,
            Outcome::InsufficientEdges => 0xffa000ff,
            Outcome::NestedOrDuplicate => 0x909090ff,
            Outcome::NotTextLike => 0xc000ffff,
            Outcome::Accepted => unreachable!(),
        }
    } else {
        match region.source {
            Source::Grayscale => 0x00ff00ff,
            Source::Icon => 0x00cfffff,
            Source::Color => 0xff00d0ff,
            Source::Underline => 0xffd000ff,
        }
    };
    let fill_color = if region.edge_pixels > 16 {
        border_color & 0xffffff80
    } else {
        border_color & 0xffffff30
    };
    let rect = if dot {
        region.rect
    } else {
        region.stroke_bounds.unwrap_or(region.rect)
    };
    NativeHint {
        x: region.display_x + f64::from(rect.x),
        y: main_height - (region.display_y + f64::from(rect.y) + f64::from(rect.height)),
        width: f64::from(rect.width),
        height: f64::from(rect.height),
        label: ptr::null(),
        selected: false,
        prefix_length: 0,
        custom_colors: true,
        debug_dot: dot,
        border_color,
        fill_color: if dot { border_color } else { fill_color },
    }
}

fn color(color: crate::font::Color) -> u32 {
    u32::from_be_bytes([color.r, color.g, color.b, color.a])
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
