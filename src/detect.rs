#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectionLimits {
    pub min_width: f64,
    pub max_width: f64,
    pub min_height: f64,
    pub max_height: f64,
}

impl Default for DetectionLimits {
    fn default() -> Self {
        Self {
            min_width: 8.0,
            max_width: 499.0,
            min_height: 4.0,
            max_height: 49.0,
        }
    }
}

#[derive(Clone, Debug)]
struct Component {
    bounds: Rect,
    edge_pixels: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    InsufficientEdges,
    TooSmall,
    TooLarge,
    NestedOrDuplicate,
    NotTextLike,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Grayscale,
    Color,
    Underline,
}

#[derive(Clone, Debug)]
pub struct DiagnosticComponent {
    pub bounds: Rect,
    // Original edge samples, chromatic strokes, or thin underline samples.
    pub edge_pixels: usize,
    pub outcome: Outcome,
    pub source: Source,
    // Underline extent including bridged gaps; not a retained pixel mask.
    pub stroke_bounds: Option<Rect>,
}

#[derive(Default)]
pub struct DetectionTrace {
    pub width: u32,
    pub height: u32,
    pub edges: Vec<u8>,
    pub color_strokes: Vec<u8>,
    pub components: Vec<DiagnosticComponent>,
}

pub struct Detection {
    pub regions: Vec<Rect>,
    pub trace: DetectionTrace,
}

#[cfg(test)]
pub fn detect_regions_from_luma(
    luma: &[u8],
    width: u32,
    height: u32,
    scale: f64,
    limits: DetectionLimits,
) -> Vec<Rect> {
    detect_with_trace(luma, width, height, scale, limits).regions
}

pub fn detect_with_trace(
    luma: &[u8],
    width: u32,
    height: u32,
    scale: f64,
    limits: DetectionLimits,
) -> Detection {
    if width == 0
        || height == 0
        || !scale.is_finite()
        || scale <= 0.0
        || luma.len() < (width as usize).saturating_mul(height as usize)
    {
        return Detection { regions: Vec::new(), trace: DetectionTrace::default() };
    }
    // Work in logical pixels so morphology and size limits behave identically
    // on integer and fractional-scale outputs.
    let logical_width = (width as f64 / scale).round().max(1.0) as u32;
    let logical_height = (height as f64 / scale).round().max(1.0) as u32;
    let normalized = resize_luma(luma, width, height, logical_width, logical_height);
    let edges = sobel_edges(&normalized, logical_width as usize, logical_height as usize);
    let dilated = dilate(
        &edges,
        logical_width as usize,
        logical_height as usize,
        3,
        5,
    );
    let components = connected_edge_components(
        &dilated,
        &edges,
        logical_width as usize,
        logical_height as usize,
    );
    let (mut rects, components) = classify_components(components, limits);
    rects.sort_by_key(|rect| (rect.y, rect.x, rect.width, rect.height));
    let regions = rects
        .into_iter()
        .map(|rect| map_rect(rect, logical_width, logical_height, width, height))
        .collect();
    Detection {
        regions,
        trace: DetectionTrace {
            width: logical_width, height: logical_height, edges, components,
            color_strokes: Vec::new(),
        },
    }
}

fn classify_components(
    components: Vec<Component>,
    limits: DetectionLimits,
) -> (Vec<Rect>, Vec<DiagnosticComponent>) {
    let mut components: Vec<_> = components.into_iter().map(|component| {
        let rect = component.bounds;
        let outcome = size_outcome(rect, component.edge_pixels, limits);
        DiagnosticComponent {
            bounds: rect, edge_pixels: component.edge_pixels, outcome, source: Source::Grayscale,
            stroke_bounds: None,
        }
    }).collect();
    let rects = filter_nested(
        components.iter().filter(|c| c.outcome == Outcome::Accepted).map(|c| c.bounds).collect(),
        1.0,
    );
    let mut accepted: std::collections::HashSet<_> = rects.iter().copied().collect();
    for component in &mut components {
        if component.outcome == Outcome::Accepted && !accepted.remove(&component.bounds) {
            component.outcome = Outcome::NestedOrDuplicate;
        }
    }
    (rects, components)
}

fn size_outcome(rect: Rect, pixels: usize, limits: DetectionLimits) -> Outcome {
    if pixels < 8 {
        Outcome::InsufficientEdges
    } else if (rect.width as f64) < limits.min_width || (rect.height as f64) < limits.min_height {
        Outcome::TooSmall
    } else if rect.width as f64 > limits.max_width || rect.height as f64 > limits.max_height {
        Outcome::TooLarge
    } else {
        Outcome::Accepted
    }
}

pub fn map_rect(rect: Rect, from_w: u32, from_h: u32, to_w: u32, to_h: u32) -> Rect {
    let x = (rect.x as u64 * to_w as u64 / from_w as u64) as u32;
    let y = (rect.y as u64 * to_h as u64 / from_h as u64) as u32;
    let right = ((rect.x as u64 + rect.width as u64) * to_w as u64)
        .div_ceil(from_w as u64)
        .min(to_w as u64) as u32;
    let bottom = ((rect.y as u64 + rect.height as u64) * to_h as u64)
        .div_ceil(from_h as u64)
        .min(to_h as u64) as u32;
    Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}

pub fn resize_luma(luma: &[u8], width: u32, height: u32, out_w: u32, out_h: u32) -> Vec<u8> {
    resize_channels(luma, width, height, out_w, out_h, |v| [v], |v| v[0])
}

pub type Rgb = [u8; 3];

pub fn luma(rgb: &[Rgb]) -> Vec<u8> {
    rgb.iter().map(|&[r, g, b]| {
        ((r as u16 * 77 + g as u16 * 150 + b as u16 * 29) / 256) as u8
    }).collect()
}

pub fn resize_rgb(rgb: &[Rgb], width: u32, height: u32, out_w: u32, out_h: u32) -> Vec<Rgb> {
    resize_channels(rgb, width, height, out_w, out_h, |v| v, |v| v)
}

fn resize_channels<T: Copy, const N: usize>(
    pixels: &[T], width: u32, height: u32, out_w: u32, out_h: u32,
    channels: impl Fn(T) -> [u8; N], pixel: impl Fn([u8; N]) -> T,
) -> Vec<T> {
    if (width, height) == (out_w, out_h) {
        return pixels.to_vec();
    }
    let mut out = vec![pixel([0; N]); out_w as usize * out_h as usize];
    // Area sampling preserves thin antialiased strokes when reducing HiDPI captures.
    for y in 0..out_h {
        let top = y as f64 * height as f64 / out_h as f64;
        let bottom = (y + 1) as f64 * height as f64 / out_h as f64;
        for x in 0..out_w {
            let left = x as f64 * width as f64 / out_w as f64;
            let right = (x + 1) as f64 * width as f64 / out_w as f64;
            let mut sum = [0.0; N];
            for sy in top.floor() as u32..(bottom.ceil() as u32).min(height) {
                for sx in left.floor() as u32..(right.ceil() as u32).min(width) {
                    let weight = (bottom.min((sy + 1) as f64) - top.max(sy as f64))
                        * (right.min((sx + 1) as f64) - left.max(sx as f64));
                    for (sum, value) in sum.iter_mut().zip(channels(pixels[(sy * width + sx) as usize])) {
                        *sum += value as f64 * weight;
                    }
                }
            }
            out[(y * out_w + x) as usize] =
                pixel(sum.map(|sum| (sum / ((right - left) * (bottom - top))).round() as u8));
        }
    }
    out
}

/// Returns logical coordinates, using the compositor's exact output dimensions.
pub fn detect_rgb_with_trace(
    rgb: &[Rgb],
    size: (u32, u32),
    logical_size: (u32, u32),
    limits: DetectionLimits,
    color_links: bool,
    underline_links: bool,
) -> Detection {
    let (width, height) = size;
    let (out_w, out_h) = logical_size;
    if width == 0 || height == 0 || out_w == 0 || out_h == 0
        || rgb.len() < width as usize * height as usize
    {
        return Detection { regions: Vec::new(), trace: DetectionTrace::default() };
    }
    // Convert before resampling to retain the primary detector's rounding.
    let normalized = resize_luma(&luma(rgb), width, height, out_w, out_h);
    let mut detection = detect_with_trace(&normalized, out_w, out_h, 1.0, limits);
    if color_links {
        let resized;
        let normalized_rgb = if size == logical_size {
            rgb
        } else {
            resized = resize_rgb(rgb, width, height, out_w, out_h);
            &resized
        };
        add_color_regions(&mut detection, normalized_rgb, limits);
    }
    if underline_links {
        add_underline_regions(&mut detection, &normalized, limits);
    }
    detection
}

fn chroma([r, g, b]: Rgb) -> u8 {
    r.max(g).max(b) - r.min(g).min(b)
}

fn hue(pixel: Rgb) -> Rgb {
    let low = *pixel.iter().min().unwrap();
    let range = chroma(pixel).max(1) as u16;
    pixel.map(|v| ((v - low) as u16 * 255 / range) as u8)
}

fn similar_hue(a: Rgb, b: Rgb) -> bool {
    a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 64)
}

fn color_distance(a: Rgb, b: Rgb) -> i16 {
    // Opponent channels ignore brightness, including equal-luminance strokes.
    let opponent = |p: Rgb| [p[0] as i16 - p[1] as i16, p[2] as i16 - p[1] as i16];
    opponent(a).into_iter().zip(opponent(b))
        .map(|(a, b)| (a - b).abs()).max().unwrap()
}

fn color_strokes(rgb: &[Rgb], width: usize, height: usize) -> Vec<u8> {
    let mut mask = vec![0; width * height];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let pixel = rgb[index];
            if chroma(pixel) < 24 {
                continue;
            }
            // A stroke has a similar background on opposite sides. One-sided
            // RGB edges would select panel borders and merge them with text.
            for distance in [2, 4] {
                let horizontal = (x >= distance && x + distance < width)
                    .then(|| (rgb[index - distance], rgb[index + distance]));
                let vertical = (y >= distance && y + distance < height)
                    .then(|| (rgb[index - distance * width], rgb[index + distance * width]));
                if [horizontal, vertical].into_iter().flatten().any(|(a, b)| {
                    color_distance(a, b) <= 16
                        && color_distance(pixel, a) >= 24
                        && color_distance(pixel, b) >= 24
                        && chroma(pixel) as u16 >= chroma(a) as u16 + 12
                        && chroma(pixel) as u16 >= chroma(b) as u16 + 12
                }) {
                    mask[index] = 1;
                    break;
                }
            }
        }
    }
    mask
}

fn near_duplicate(a: Rect, b: Rect) -> bool {
    a.x.abs_diff(b.x) <= 3 && a.y.abs_diff(b.y) <= 3
        && (a.x + a.width).abs_diff(b.x + b.width) <= 3
        && (a.y + a.height).abs_diff(b.y + b.height) <= 3
        && (a.width as u64 * a.height as u64).min(b.width as u64 * b.height as u64) * 100
            >= (a.width as u64 * a.height as u64).max(b.width as u64 * b.height as u64) * 65
}

// Index top-left corners: near duplicates cannot be more than one cell away.
// Containment alone must not discard inline targets inside larger components.
struct SupplementalTargets(std::collections::HashMap<(u32, u32), Vec<Rect>>);

impl SupplementalTargets {
    fn new(regions: &[Rect]) -> Self {
        let mut index = Self(std::collections::HashMap::new());
        for &rect in regions {
            index.insert(rect);
        }
        index
    }

    fn insert(&mut self, rect: Rect) {
        self.0.entry((rect.x / 8, rect.y / 8)).or_default().push(rect);
    }

    fn record(&mut self, detection: &mut Detection, mut component: DiagnosticComponent) {
        if component.outcome == Outcome::Accepted {
            let rect = component.bounds;
            let (cx, cy) = (rect.x / 8, rect.y / 8);
            let duplicate = (cy.saturating_sub(1)..=cy + 1).any(|y| {
                (cx.saturating_sub(1)..=cx + 1).any(|x| {
                    self.0.get(&(x, y)).is_some_and(|rects|
                        rects.iter().any(|&other| near_duplicate(rect, other)))
                })
            });
            if duplicate {
                component.outcome = Outcome::NestedOrDuplicate;
            } else {
                self.insert(rect);
                detection.regions.push(rect);
            }
        }
        detection.trace.components.push(component);
    }
}

fn add_color_regions(detection: &mut Detection, rgb: &[Rgb], limits: DetectionLimits) {
    let width = detection.trace.width as usize;
    let height = detection.trace.height as usize;
    let mask = color_strokes(rgb, width, height);
    let mut visited = vec![false; mask.len()];
    let mut queue = std::collections::VecDeque::new();
    let mut targets = SupplementalTargets::new(&detection.regions);
    for start in 0..mask.len() {
        if mask[start] == 0 || visited[start] {
            continue;
        }
        let anchor = hue(rgb[start]);
        visited[start] = true;
        queue.push_back(start);
        let (mut left, mut right) = (start % width, start % width);
        let (mut top, mut bottom) = (start / width, start / width);
        let mut pixels = 0;
        while let Some(index) = queue.pop_front() {
            let (x, y) = (index % width, index / width);
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            pixels += 1;
            // Bridge inter-letter gaps, not neutral paragraph strokes. An
            // underline can connect letters, but is not required as evidence.
            for yy in y.saturating_sub(3)..=(y + 3).min(height - 1) {
                for xx in x.saturating_sub(5)..=(x + 5).min(width - 1) {
                    let next = yy * width + xx;
                    if mask[next] != 0 && !visited[next] && similar_hue(anchor, hue(rgb[next])) {
                        visited[next] = true;
                        queue.push_back(next);
                    }
                }
            }
        }
        let bounds = Rect {
            x: left as u32, y: top as u32,
            width: (right - left + 1) as u32, height: (bottom - top + 1) as u32,
        };
        let mut component = DiagnosticComponent {
            bounds, edge_pixels: pixels, outcome: Outcome::Accepted, source: Source::Color,
            stroke_bounds: None,
        };
        let size = size_outcome(bounds, pixels, limits);
        component.outcome = if size != Outcome::Accepted {
            size
        } else if !text_like(rgb, width, bounds, anchor) {
            Outcome::NotTextLike
        } else {
            Outcome::Accepted
        };
        targets.record(detection, component);
    }
    detection.trace.color_strokes = mask;
    detection.regions.sort_by_key(|rect| (rect.y, rect.x, rect.width, rect.height));
}

fn thin_stroke(luma: &[u8], width: usize, x: usize, y: usize) -> usize {
    let index = y * width + x;
    let background = luma[index - width];
    let foreground = luma[index];
    if background.abs_diff(foreground) < 24 {
        return 0;
    }
    for thickness in 1..=3 {
        let below = luma[index + thickness * width];
        if background.abs_diff(below) <= 16 {
            return thickness;
        }
        if (below < background) != (foreground < background) {
            break;
        }
    }
    0
}

fn underline_text_bounds(
    luma: &[u8], width: usize, height: usize, line: Rect, background: u8, dark: bool,
) -> Option<Rect> {
    let (left, right, y) = (line.x as usize, (line.x + line.width) as usize, line.y as usize);
    let ink = |x: usize, y: usize| {
        let value = luma[y * width + x];
        value.abs_diff(background) >= 24 && (value < background) == dark
    };
    let row = |yy: usize| {
        let (mut count, mut runs, mut previous) = (0, 0, false);
        for x in left..right {
            let present = ink(x, yy);
            count += usize::from(present);
            runs += usize::from(present && !previous);
            previous = present;
        }
        (count, runs)
    };
    let (mut top, mut bottom, mut blank, mut split_rows, mut samples) = (y, y, 0, 0, 0);
    let (mut first, mut last) = (right, left);
    for yy in (y.saturating_sub(36)..y).rev() {
        let (count, runs) = row(yy);
        if count == 0 {
            blank += 1;
            if (top == y && blank > 4) || (top != y && blank >= 2) {
                break;
            }
            continue;
        }
        if top == y {
            bottom = yy + 1;
        }
        top = yy;
        blank = 0;
        samples += count;
        split_rows += usize::from(runs >= 2);
        for x in left..right {
            if ink(x, yy) {
                first = first.min(x);
                last = last.max(x);
            }
        }
    }
    let band_height = bottom - top;
    if !(5..=32).contains(&band_height) || split_rows < 3
        || samples * 100 < (right - left) * band_height * 10
        || samples * 100 > (right - left) * band_height * 65
        || first > left + (right - left) / 5 || last + 1 < right - (right - left) / 5
    {
        return None;
    }
    // A strike has substantial glyph bodies below the line. Sparse descenders
    // are allowed, but not a second half of the text band.
    let below = y + line.height as usize;
    let (mut blank_below, mut body_below) = (0, 0);
    for yy in below..(below + 5).min(height) {
        let (count, _) = row(yy);
        blank_below = if count == 0 { blank_below + 1 } else { 0 };
        if blank_below >= 2 {
            break;
        }
        body_below += usize::from(count * 100 >= (right - left) * 20);
    }
    if body_below >= 2 {
        return None;
    }
    // Reject side walls reaching the line, including corners omitted by the
    // ridge test. Normal glyph stems stop above the underline's separation.
    for x in left.saturating_sub(3)..(left + 3).min(width) {
        if (y.saturating_sub(5)..y).all(|yy| ink(x, yy)) {
            return None;
        }
    }
    for x in right.saturating_sub(3)..(right + 3).min(width) {
        if (y.saturating_sub(5)..y).all(|yy| ink(x, yy)) {
            return None;
        }
    }
    Some(Rect { x: line.x, y: top as u32, width: line.width,
        height: line.y + line.height - top as u32 })
}

fn add_underline_regions(detection: &mut Detection, luma: &[u8], limits: DetectionLimits) {
    let (width, height) = (detection.trace.width as usize, detection.trace.height as usize);
    if width < 12 || height < 8 {
        return;
    }
    let mut targets = SupplementalTargets::new(&detection.regions);
    // One row of ridge thicknesses, not another full capture or debug mask.
    let mut strokes = vec![0usize; width];
    for y in 1..height - 3 {
        for (x, stroke) in strokes.iter_mut().enumerate() {
            *stroke = thin_stroke(luma, width, x, y);
        }
        let mut x = 0;
        while x < width {
            if strokes[x] == 0 {
                x += 1;
                continue;
            }
            let left = x;
            let background = luma[(y - 1) * width + x];
            let dark = luma[y * width + x] < background;
            let (mut right, mut pixels, mut covered, mut thickness, mut gap) = (x, 0, 0, 0, 0);
            while x < width {
                let present = strokes[x] != 0
                    && luma[(y - 1) * width + x].abs_diff(background) <= 16
                    && (luma[y * width + x] < background) == dark;
                if present {
                    right = x + 1;
                    covered += 1;
                    pixels += strokes[x];
                    thickness = thickness.max(strokes[x]);
                    gap = 0;
                } else {
                    gap += 1;
                    if gap > 3 {
                        break;
                    }
                }
                x += 1;
            }
            let span = right - left;
            // Fixed geometry bounds keep verification linear with a small
            // constant, even when configured control size limits are enormous.
            if !(12..=1024).contains(&span) || covered * 100 < span * 75 {
                continue;
            }
            let line = Rect { x: left as u32, y: y as u32,
                width: span as u32, height: thickness as u32 };
            let text = underline_text_bounds(luma, width, height, line, background, dark);
            let bounds = text.unwrap_or(line);
            let size = size_outcome(bounds, pixels, limits);
            targets.record(detection, DiagnosticComponent {
                bounds, edge_pixels: pixels, source: Source::Underline, stroke_bounds: Some(line),
                outcome: if text.is_none() { Outcome::NotTextLike }
                    else if size != Outcome::Accepted { size }
                    else { Outcome::Accepted },
            });
        }
    }
    detection.regions.sort_by_key(|rect| (rect.y, rect.x, rect.width, rect.height));
}

fn text_like(rgb: &[Rgb], width: usize, bounds: Rect, anchor: Rgb) -> bool {
    let mut foreground = 0;
    let mut split_rows = 0;
    for y in bounds.y..bounds.y + bounds.height {
        let mut previous = false;
        let mut runs = 0;
        for x in bounds.x..bounds.x + bounds.width {
            let pixel = rgb[y as usize * width + x as usize];
            let present = chroma(pixel) >= 24 && similar_hue(anchor, hue(pixel));
            foreground += usize::from(present);
            runs += usize::from(present && !previous);
            previous = present;
        }
        split_rows += usize::from(runs >= 2);
    }
    // Flat fills, isolated lines, and solid badges are not text evidence.
    foreground * 100 <= bounds.width as usize * bounds.height as usize * 70 && split_rows >= 3
}

fn sobel_edges(luma: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut edges = vec![0; width * height];
    let mut weak = vec![false; width * height];
    let mut queue = std::collections::VecDeque::new();
    if width < 3 || height < 3 {
        return edges;
    }
    let pixel = |x: usize, y: usize| luma[y * width + x] as i32;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let gx = -pixel(x - 1, y - 1) + pixel(x + 1, y - 1) - 2 * pixel(x - 1, y)
                + 2 * pixel(x + 1, y)
                - pixel(x - 1, y + 1)
                + pixel(x + 1, y + 1);
            let gy = pixel(x - 1, y - 1) + 2 * pixel(x, y - 1) + pixel(x + 1, y - 1)
                - pixel(x - 1, y + 1)
                - 2 * pixel(x, y + 1)
                - pixel(x + 1, y + 1);
            let magnitude = gx * gx + gy * gy;
            weak[y * width + x] = magnitude >= 60 * 60;
            if magnitude >= 160 * 160 {
                edges[y * width + x] = 1;
                queue.push_back((x, y));
            }
        }
    }
    // Hysteresis retains faint antialiased edges connected to strong strokes,
    // without accepting every weak gradient in wallpapers and shadows.
    while let Some((x, y)) = queue.pop_front() {
        for yy in y.saturating_sub(1)..=(y + 1).min(height - 1) {
            for xx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                let index = yy * width + xx;
                if weak[index] && edges[index] == 0 {
                    edges[index] = 1;
                    queue.push_back((xx, yy));
                }
            }
        }
    }
    edges
}

fn dilate(mask: &[u8], width: usize, height: usize, kernel_h: usize, kernel_w: usize) -> Vec<u8> {
    let mut out = vec![0; width * height];
    let radius_y = kernel_h / 2;
    let radius_x = kernel_w / 2;
    for y in 0..height {
        let y0 = y.saturating_sub(radius_y);
        let y1 = (y + radius_y).min(height - 1);
        for x in 0..width {
            let x0 = x.saturating_sub(radius_x);
            let x1 = (x + radius_x).min(width - 1);
            'found: for yy in y0..=y1 {
                for xx in x0..=x1 {
                    if mask[yy * width + xx] != 0 {
                        out[y * width + x] = 1;
                        break 'found;
                    }
                }
            }
        }
    }
    out
}

fn connected_edge_components(
    mask: &[u8],
    edges: &[u8],
    width: usize,
    height: usize,
) -> Vec<Component> {
    let mut visited = vec![false; width * height];
    let mut queue = std::collections::VecDeque::new();
    let mut components = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if visited[index] || mask[index] == 0 {
                continue;
            }
            visited[index] = true;
            queue.push_back((x, y));
            let mut min_x = width;
            let mut max_x = 0;
            let mut min_y = height;
            let mut max_y = 0;
            let mut edge_pixels = 0;
            while let Some((cx, cy)) = queue.pop_front() {
                if edges[cy * width + cx] != 0 {
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);
                    edge_pixels += 1;
                }
                for ny in cy.saturating_sub(1)..=(cy + 1).min(height - 1) {
                    for nx in cx.saturating_sub(1)..=(cx + 1).min(width - 1) {
                        let neighbor = ny * width + nx;
                        if visited[neighbor] || mask[neighbor] == 0 {
                            continue;
                        }
                        visited[neighbor] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
            components.push(Component {
                bounds: Rect {
                    x: min_x as u32,
                    y: min_y as u32,
                    width: (max_x - min_x + 1) as u32,
                    height: (max_y - min_y + 1) as u32,
                },
                edge_pixels,
            });
        }
    }
    components
}

fn filter_nested(rects: Vec<Rect>, scale: f64) -> Vec<Rect> {
    let mut rects = rects;
    // Visit enclosing contours first, independent of their discovery order.
    rects.sort_by_key(|r| {
        (
            std::cmp::Reverse(r.width as u64 * r.height as u64),
            r.y,
            r.x,
            r.width,
            r.height,
        )
    });
    rects.dedup();
    let center_limit = 8.0 * scale;
    let inner_height_limit = 6.0 * scale;
    let square_limit = 40.0 * scale;
    let square_delta = 5.0 * scale;
    let mut filtered = vec![false; rects.len()];
    let parents = parent_indices(&rects);
    for (index, rect) in rects.iter().enumerate() {
        let Some(parent_index) = parents[index] else {
            continue;
        };
        if filtered[parent_index] {
            filtered[index] = true;
            continue;
        }
        if rect.height as f64 <= inner_height_limit {
            filtered[index] = true;
            continue;
        }
        let parent = rects[parent_index];
        let (cx, cy) = center(rect);
        let (px, py) = center(&parent);
        if (cx - px).abs() < center_limit && (cy - py).abs() < center_limit {
            filtered[index] = true;
            continue;
        }
        if ((parent.width as f64) - (parent.height as f64)).abs() < square_delta
            && parent.width as f64 <= square_limit
            && parent.height as f64 <= square_limit
        {
            filtered[index] = true;
        }
    }
    rects
        .into_iter()
        .enumerate()
        .filter_map(|(index, rect)| (!filtered[index]).then_some(rect))
        .collect()
}

fn parent_indices(rects: &[Rect]) -> Vec<Option<usize>> {
    let mut parents = vec![None; rects.len()];
    for (index, rect) in rects.iter().enumerate() {
        let mut best: Option<(usize, u64)> = None;
        for (candidate_index, candidate) in rects.iter().enumerate() {
            if index == candidate_index || !contains(candidate, rect) {
                continue;
            }
            let area = candidate.width as u64 * candidate.height as u64;
            if best.is_none_or(|(_, current_area)| area < current_area) {
                best = Some((candidate_index, area));
            }
        }
        parents[index] = best.map(|(candidate, _)| candidate);
    }
    parents
}

fn contains(outer: &Rect, inner: &Rect) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.width >= inner.x + inner.width
        && outer.y + outer.height >= inner.y + inner.height
        && (outer.x != inner.x
            || outer.y != inner.y
            || outer.width != inner.width
            || outer.height != inner.height)
}

fn center(rect: &Rect) -> (f64, f64) {
    (
        rect.x as f64 + rect.width as f64 / 2.0,
        rect.y as f64 + rect.height as f64 / 2.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_text(pixels: &mut [Rgb], width: u32, x: i32, y: i32, text: &str, color: Rgb) {
        use crate::font::{Canvas, Color};
        let mut bgra: Vec<_> = pixels.iter().flat_map(|&[r, g, b]| [b, g, r, 255]).collect();
        let height = pixels.len() as u32 / width;
        Canvas::new(&mut bgra, width, height).draw_text(
            x, y, text, 2, Color::rgba(color[0], color[1], color[2], 255),
        );
        for (pixel, bgra) in pixels.iter_mut().zip(bgra.chunks_exact(4)) {
            *pixel = [bgra[2], bgra[1], bgra[0]];
        }
    }

    fn rgb_targets(rgb: &[Rgb], width: u32, height: u32, enabled: bool) -> Detection {
        detect_rgb_with_trace(rgb, (width, height), (width, height), DetectionLimits::default(), enabled, false)
    }

    fn accepted_color(detection: &Detection) -> Vec<Rect> {
        detection.trace.components.iter()
            .filter(|c| c.source == Source::Color && c.outcome == Outcome::Accepted)
            .map(|c| c.bounds).collect()
    }

    fn underline_targets(rgb: &[Rgb], size: (u32, u32), logical: (u32, u32)) -> Detection {
        detect_rgb_with_trace(rgb, size, logical, DetectionLimits::default(), false, true)
    }

    fn accepted_underlines(detection: &Detection) -> Vec<Rect> {
        detection.trace.components.iter()
            .filter(|c| c.source == Source::Underline && c.outcome == Outcome::Accepted)
            .map(|c| c.bounds).collect()
    }

    fn underline(rgb: &mut [Rgb], width: usize, left: usize, right: usize, y: usize, color: Rgb) {
        rgb[y * width + left..y * width + right].fill(color);
    }

    #[test]
    fn underline_same_color_inline_light_dark_antialias_and_skip_ink() {
        let expected = Rect { x: 60, y: 20, width: 46, height: 17 };
        for (bg, fg) in [(245, 70), (20, 220)] {
            for style in 0..5 {
                let mut rgb = vec![[bg; 3]; 200 * 80];
                rgb_text(&mut rgb, 200, 12, 20, "TEXTLINKTEXT", [fg; 3]);
                let line_color = if style == 1 { ((bg as u16 + fg as u16) / 2) as u8 } else { fg };
                underline(&mut rgb, 200, 60, 106, 36, [line_color; 3]);
                if style == 2 {
                    for x in [70, 71, 72, 94, 95, 96] {
                        rgb[36 * 200 + x] = [bg; 3];
                    }
                    for y in 33..39 {
                        rgb[y * 200 + 71] = [fg; 3];
                        rgb[y * 200 + 95] = [fg; 3];
                    }
                }
                if style == 3 {
                    underline(&mut rgb, 200, 60, 106, 37, [fg; 3]);
                }
                if style == 4 {
                    let fringe = if bg > fg { bg - 30 } else { bg + 30 };
                    underline(&mut rgb, 200, 60, 106, 35, [fringe; 3]);
                    underline(&mut rgb, 200, 60, 106, 37, [fringe; 3]);
                }
                let expected = Rect { height: if style >= 3 { 18 } else { 17 }, ..expected };
                let off = rgb_targets(&rgb, 200, 80, false);
                let detection = underline_targets(&rgb, (200, 80), (200, 80));
                assert_eq!(accepted_underlines(&detection), [expected],
                    "bg={bg}, style={style}: {:?}", detection.trace.components);
                assert_eq!(center(&accepted_underlines(&detection)[0]),
                    (83.0, if style >= 3 { 29.0 } else { 28.5 }));
                assert!(off.regions.iter().any(|r| contains(r, &expected)));
                assert!(off.regions.iter().all(|r| detection.regions.contains(r)));
                assert_trace_consistent(&detection);
            }
        }
    }

    #[test]
    fn underline_adjacent_links_and_multiline_paragraph_stay_bounded() {
        let mut rgb = vec![[245; 3]; 240 * 100];
        for y in [20, 40, 60] {
            rgb_text(&mut rgb, 240, 12, y, "TEXTLINKLINKTEXT", [70; 3]);
            underline(&mut rgb, 240, 60, 106, y as usize + 16, [70; 3]);
            underline(&mut rgb, 240, 110, 154, y as usize + 16, [70; 3]);
        }
        let detection = underline_targets(&rgb, (240, 100), (240, 100));
        let expected: Vec<_> = [20, 40, 60].into_iter().flat_map(|y| [
            Rect { x: 60, y, width: 46, height: 17 },
            Rect { x: 110, y, width: 44, height: 17 },
        ]).collect();
        assert_eq!(accepted_underlines(&detection), expected, "{:?}", detection.trace.components);
        for (rect, expected) in accepted_underlines(&detection).iter().zip(expected) {
            assert_eq!(center(rect), center(&expected));
        }
        assert_trace_consistent(&detection);
    }

    #[test]
    fn underline_rejects_strikes_dividers_box_borders_fills_and_noise() {
        for case in 0..7 {
            let mut rgb = vec![[245; 3]; 240 * 100];
            match case {
                0 => {
                    rgb_text(&mut rgb, 240, 60, 20, "LINK", [70; 3]);
                    underline(&mut rgb, 240, 60, 106, 27, [70; 3]);
                }
                1 => underline(&mut rgb, 240, 20, 220, 60, [70; 3]),
                2 => {
                    rgb_text(&mut rgb, 240, 60, 20, "LINK", [70; 3]);
                    underline(&mut rgb, 240, 20, 220, 36, [70; 3]);
                }
                3 => {
                    rgb_text(&mut rgb, 240, 60, 20, "LINK", [70; 3]);
                    for y in 16..37 {
                        rgb[y * 240 + 56] = [70; 3];
                        rgb[y * 240 + 109] = [70; 3];
                    }
                    for y in [16, 36] {
                        underline(&mut rgb, 240, 56, 110, y, [70; 3]);
                    }
                }
                4 => {
                    for y in 20..60 {
                        underline(&mut rgb, 240, 20, 220, y, [70; 3]);
                    }
                }
                5 => {
                    let mut state = 17u32;
                    for pixel in &mut rgb {
                        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                        *pixel = [(state >> 24) as u8; 3];
                    }
                }
                _ => {
                    for (i, pixel) in rgb.iter_mut().enumerate() {
                        *pixel = [230 + (i % 15) as u8; 3];
                    }
                }
            }
            let detection = underline_targets(&rgb, (240, 100), (240, 100));
            assert!(accepted_underlines(&detection).is_empty(),
                "case={case}: {:?}", detection.trace.components);
            if (1..=3).contains(&case) {
                assert!(detection.trace.components.iter().any(|c|
                    c.source == Source::Underline && c.outcome == Outcome::NotTextLike
                        && c.stroke_bounds.is_some()));
            }
            assert_trace_consistent(&detection);
        }
    }

    #[test]
    fn underline_scale_limits_opt_out_and_source_precedence() {
        let mut rgb = vec![[245; 3]; 200 * 80];
        rgb_text(&mut rgb, 200, 12, 20, "TEXT", [90; 3]);
        rgb_text(&mut rgb, 200, 60, 20, "LINK", [30, 90, 210]);
        rgb_text(&mut rgb, 200, 108, 20, "TEXT", [90; 3]);
        underline(&mut rgb, 200, 60, 106, 36, [30, 90, 210]);
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let size = ((200.0 * scale) as u32, (80.0 * scale) as u32);
            let scaled = resize_rgb(&rgb, 200, 80, size.0, size.1);
            let detection = underline_targets(&scaled, size, (200, 80));
            let links = accepted_underlines(&detection);
            assert_eq!(links.len(), 1, "scale={scale}: {:?}", detection.trace.components);
            let rect = links[0];
            assert!(rect.x.abs_diff(60) <= 1 && rect.y.abs_diff(20) <= 1);
            assert!(rect.width.abs_diff(46) <= 2 && rect.height.abs_diff(17) <= 2);
            let (cx, cy) = center(&rect);
            assert!((cx - 83.0).abs() <= 1.0 && (cy - 28.5).abs() <= 1.0);
            let physical = map_rect(rect, 200, 80, size.0, size.1);
            assert!(covers(&physical, (83.0 * scale) as u32, (28.5 * scale) as u32));
            let both = detect_rgb_with_trace(&scaled, size, (200, 80), DetectionLimits::default(), true, true);
            let color_only = detect_rgb_with_trace(&scaled, size, (200, 80), DetectionLimits::default(), true, false);
            assert_eq!(both.regions, color_only.regions);
            assert_eq!(both.trace.color_strokes, color_only.trace.color_strokes);
            assert_eq!(both.trace.edges, color_only.trace.edges);
            assert!(both.trace.components.iter().any(|c|
                c.source == Source::Underline && c.outcome == Outcome::NestedOrDuplicate));
            assert!(color_only.trace.components.iter().all(|c| c.source != Source::Underline));
            assert_trace_consistent(&both);
        }
        for (limits, outcome) in [
            (DetectionLimits { min_width: 47.0, ..DetectionLimits::default() }, Outcome::TooSmall),
            (DetectionLimits { max_width: 45.0, ..DetectionLimits::default() }, Outcome::TooLarge),
            (DetectionLimits { max_height: 16.0, ..DetectionLimits::default() }, Outcome::TooLarge),
        ] {
            let detection = detect_rgb_with_trace(&rgb, (200, 80), (200, 80), limits, false, true);
            assert!(detection.trace.components.iter().any(|c| c.source == Source::Underline
                && c.outcome == outcome && c.bounds == Rect { x: 60, y: 20, width: 46, height: 17 }));
            assert_trace_consistent(&detection);
        }
        let mut isolated = vec![[245; 3]; 160 * 80];
        rgb_text(&mut isolated, 160, 40, 20, "LINK", [70; 3]);
        underline(&mut isolated, 160, 40, 86, 36, [70; 3]);
        let baseline = rgb_targets(&isolated, 160, 80, false);
        let detection = underline_targets(&isolated, (160, 80), (160, 80));
        assert_eq!(detection.regions, baseline.regions);
        assert_eq!(detection.regions, [Rect { x: 39, y: 19, width: 48, height: 19 }]);
        assert!(detection.trace.components.iter().any(|c|
            c.source == Source::Underline && c.outcome == Outcome::NestedOrDuplicate));
        assert_trace_consistent(&detection);
    }

    fn assert_trace_consistent(detection: &Detection) {
        let trace = &detection.trace;
        for component in &trace.components {
            assert_eq!(component.stroke_bounds.is_some(), component.source == Source::Underline);
            if let Some(line) = component.stroke_bounds {
                assert!(component.edge_pixels <= (line.width * line.height) as usize);
                assert_eq!(component.bounds.x, line.x);
                assert_eq!(component.bounds.width, line.width);
                assert_eq!(component.bounds.y + component.bounds.height, line.y + line.height);
            }
        }
        for (source, mask) in [(Source::Grayscale, &trace.edges), (Source::Color, &trace.color_strokes)] {
            assert_eq!(mask.iter().map(|&v| v as usize).sum::<usize>(),
                trace.components.iter().filter(|c| c.source == source)
                    .map(|c| c.edge_pixels).sum::<usize>());
        }
        let mut accepted: Vec<_> = trace.components.iter()
            .filter(|c| c.outcome == Outcome::Accepted).map(|c| c.bounds).collect();
        accepted.sort_by_key(|r| (r.y, r.x, r.width, r.height));
        assert_eq!(accepted, detection.regions);
    }

    #[test]
    fn color_inline_links_survive_merged_equal_luminance_paragraphs() {
        for background in [[245; 3], [20; 3], [235, 240, 245]] {
            for color in [[30, 90, 210], [190, 50, 130], [20, 140, 70], [240, 170, 60]] {
                for underline in [false, true] {
                    let mut rgb = vec![background; 200 * 80];
                    let neutral = [luma(&[color])[0]; 3];
                    rgb_text(&mut rgb, 200, 12, 20, "TEXT", neutral);
                    rgb_text(&mut rgb, 200, 60, 20, "LINK", color);
                    rgb_text(&mut rgb, 200, 108, 20, "TEXT", neutral);
                    if underline {
                        for x in 60..106 {
                            rgb[36 * 200 + x] = color;
                        }
                    }
                    let baseline = rgb_targets(&rgb, 200, 80, false);
                    let detection = rgb_targets(&rgb, 200, 80, true);
                    let expected = Rect { x: 60, y: 20, width: 46, height: if underline { 17 } else { 14 } };
                    assert_eq!(accepted_color(&detection), [expected],
                        "background={background:?}, color={color:?}, underline={underline}: {:?}",
                        detection.trace.components);
                    assert_eq!(center(&expected), (83.0, if underline { 28.5 } else { 27.0 }));
                    assert!(baseline.regions.iter().any(|r| contains(r, &expected)));
                    assert!(baseline.regions.iter().all(|r| detection.regions.contains(r)));
                    assert_trace_consistent(&detection);
                }
            }
        }
    }

    #[test]
    fn color_detects_strokes_even_without_luminance_edges() {
        let color = [30, 90, 210];
        let mut rgb = vec![[luma(&[color])[0]; 3]; 160 * 60];
        rgb_text(&mut rgb, 160, 40, 20, "LINK", color);
        let detection = rgb_targets(&rgb, 160, 60, true);
        assert!(detection.trace.edges.iter().all(|&v| v == 0));
        assert_eq!(detection.regions, [Rect { x: 40, y: 20, width: 46, height: 14 }]);
        assert_trace_consistent(&detection);
    }

    #[test]
    fn color_separates_nearby_links_and_hues_without_duplicates() {
        for (second_x, second_color) in [(108, [190, 50, 130]), (112, [30, 90, 210])] {
            let mut rgb = vec![[245; 3]; 240 * 80];
            rgb_text(&mut rgb, 240, 12, 20, "TEXT", [90; 3]);
            rgb_text(&mut rgb, 240, 60, 20, "LINK", [30, 90, 210]);
            rgb_text(&mut rgb, 240, second_x, 20, "LINK", second_color);
            rgb_text(&mut rgb, 240, second_x + 48, 20, "TEXT", [90; 3]);
            let detection = rgb_targets(&rgb, 240, 80, true);
            let links = accepted_color(&detection);
            assert_eq!(links, [
                Rect { x: 60, y: 20, width: 46, height: 14 },
                Rect { x: second_x as u32, y: 20, width: 46, height: 14 },
            ], "{:?}", detection.trace.components);
            assert!(!links.iter().any(|r| r.width > 46));
            assert_trace_consistent(&detection);
        }
        let mut rgb = vec![[245; 3]; 160 * 80];
        rgb_text(&mut rgb, 160, 40, 20, "LINK", [30, 90, 210]);
        let baseline = rgb_targets(&rgb, 160, 80, false);
        let detection = rgb_targets(&rgb, 160, 80, true);
        assert_eq!(detection.regions, baseline.regions);
        assert_eq!(detection.regions, [Rect { x: 39, y: 19, width: 48, height: 16 }]);
        assert!(detection.trace.components.iter().any(|c|
            c.source == Source::Color && c.outcome == Outcome::NestedOrDuplicate));
        assert_trace_consistent(&detection);
    }

    #[test]
    fn color_neutral_opt_out_noise_and_flat_panels() {
        let mut rgb = vec![[240; 3]; 320 * 160];
        rgb_text(&mut rgb, 320, 20, 20, "NEUTRAL TEXT", [80; 3]);
        let baseline = rgb_targets(&rgb, 320, 160, false);
        let detection = rgb_targets(&rgb, 320, 160, true);
        assert_eq!(detection.regions, baseline.regions);
        assert!(detection.trace.color_strokes.iter().all(|&v| v == 0));
        assert_trace_consistent(&detection);
        assert!(baseline.trace.color_strokes.is_empty());
        assert_eq!(baseline.regions, detect_with_trace(&luma(&rgb), 320, 160, 1.0,
            DetectionLimits::default()).regions);

        for (i, pixel) in rgb.iter_mut().enumerate() {
            *pixel = [240 + (i % 5) as u8, 240, 240];
        }
        for (x, y) in [(12, 12), (30, 12), (50, 12)] {
            rgb[y * 320 + x] = [20, 80, 220];
        }
        for y in 50..140 {
            for x in 20..300 {
                rgb[y * 320 + x] = [30, 90, 210];
            }
        }
        // A small flat strip can seed strokes, but must fail text shape.
        for y in 24..28 {
            for x in 80..120 {
                rgb[y * 320 + x] = [180, 40, 100];
            }
        }
        let detection = rgb_targets(&rgb, 320, 160, true);
        assert!(accepted_color(&detection).is_empty(), "{:?}", detection.trace.components);
        assert!(detection.trace.components.iter().any(|c|
            c.source == Source::Color && c.outcome == Outcome::NotTextLike));
        assert!(detection.trace.components.iter().any(|c|
            c.source == Source::Color && c.outcome == Outcome::InsufficientEdges));
        assert_trace_consistent(&detection);

        let mut state = 17u32;
        for pixel in &mut rgb {
            *pixel = std::array::from_fn(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            });
        }
        let noise = rgb_targets(&rgb, 320, 160, true);
        assert!(accepted_color(&noise).is_empty(), "{:?}", accepted_color(&noise));
        assert_trace_consistent(&noise);
    }

    #[test]
    fn color_size_rejections_and_invalid_input_are_traced() {
        let mut rgb = vec![[245; 3]; 160 * 60];
        rgb_text(&mut rgb, 160, 40, 20, "LINK", [30, 90, 210]);
        for (limits, expected) in [
            (DetectionLimits { min_width: 50.0, ..DetectionLimits::default() }, Outcome::TooSmall),
            (DetectionLimits { max_width: 40.0, ..DetectionLimits::default() }, Outcome::TooLarge),
        ] {
            let detection = detect_rgb_with_trace(&rgb, (160, 60), (160, 60), limits, true, false);
            let color: Vec<_> = detection.trace.components.iter().filter(|c| c.source == Source::Color).collect();
            assert_eq!(color.len(), 1);
            assert_eq!(color[0].outcome, expected);
            assert_eq!(color[0].bounds, Rect { x: 40, y: 20, width: 46, height: 14 });
            assert_trace_consistent(&detection);
        }
        for (rgb, size, logical_size) in [
            (&[][..], (10, 10), (10, 10)),
            (&rgb[..], (0, 60), (160, 60)),
            (&rgb[..], (160, 60), (160, 0)),
        ] {
            let detection = detect_rgb_with_trace(rgb, size, logical_size, DetectionLimits::default(), true, true);
            assert!(detection.regions.is_empty());
            assert!(detection.trace.components.is_empty());
        }
    }

    #[test]
    fn color_logical_bounds_and_centers_survive_fractional_resampling() {
        let mut rgb = vec![[245; 3]; 200 * 80];
        rgb_text(&mut rgb, 200, 12, 20, "TEXT", [90; 3]);
        rgb_text(&mut rgb, 200, 60, 20, "LINK", [30, 90, 210]);
        rgb_text(&mut rgb, 200, 108, 20, "TEXT", [90; 3]);
        let expected = Rect { x: 60, y: 20, width: 46, height: 14 };
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let (width, height) = ((200.0 * scale) as u32, (80.0 * scale) as u32);
            let scaled = resize_rgb(&rgb, 200, 80, width, height);
            let detection = detect_rgb_with_trace(&scaled, (width, height), (200, 80),
                DetectionLimits::default(), true, false);
            let disabled = detect_rgb_with_trace(&scaled, (width, height), (200, 80),
                DetectionLimits::default(), false, false);
            let baseline_luma = resize_luma(&luma(&scaled), width, height, 200, 80);
            let baseline = detect_with_trace(&baseline_luma, 200, 80, 1.0, DetectionLimits::default());
            assert_eq!(disabled.regions, baseline.regions);
            assert_eq!(disabled.trace.edges, baseline.trace.edges);
            assert!(disabled.trace.color_strokes.is_empty());
            let links = accepted_color(&detection);
            assert_eq!(links.len(), 1, "scale={scale}: {links:?}");
            let rect = links[0];
            assert!(rect.x.abs_diff(expected.x) <= 1 && rect.y.abs_diff(expected.y) <= 1);
            assert!(rect.width.abs_diff(expected.width) <= 2 && rect.height.abs_diff(expected.height) <= 2);
            let (x, y) = center(&rect);
            assert!((x - 83.0).abs() <= 1.0 && (y - 27.0).abs() <= 1.0);
            let physical = map_rect(rect, 200, 80, width, height);
            assert!(covers(&physical, (83.0 * scale) as u32, (27.0 * scale) as u32));
            assert_trace_consistent(&detection);
        }
    }

    #[test]
    #[ignore = "synthetic color pass timing; run in release mode with nocapture"]
    fn color_snapshot_cost() {
        let mut rgb = vec![[245; 3]; 1920 * 1080];
        // Construct one row once, then repeat it without storing a screenshot.
        let mut row = vec![[245; 3]; 1920 * 40];
        for x in (12..1800).step_by(180) {
            rgb_text(&mut row, 1920, x, 10, "TEXT", [90; 3]);
            rgb_text(&mut row, 1920, x + 48, 10, "LINK", [30, 90, 210]);
            rgb_text(&mut row, 1920, x + 96, 10, "TEXT", [90; 3]);
        }
        for chunk in rgb.chunks_exact_mut(row.len()) {
            chunk.copy_from_slice(&row);
        }
        for enabled in [false, true] {
            let start = std::time::Instant::now();
            let detection = rgb_targets(&rgb, 1920, 1080, enabled);
            eprintln!("1920x1080 color_links={enabled}: {:?}, {} targets, {} color targets, {} retained mask bytes, {} metadata bytes",
                start.elapsed(), detection.regions.len(), accepted_color(&detection).len(),
                detection.trace.edges.len() + detection.trace.color_strokes.len(),
                detection.trace.components.capacity() * std::mem::size_of::<DiagnosticComponent>());
            assert_eq!(accepted_color(&detection).len(), if enabled { 270 } else { 0 });
            assert_trace_consistent(&detection);
        }
    }

    #[test]
    #[ignore = "synthetic underline and combined pass timing; run in release mode with nocapture"]
    fn underline_snapshot_cost() {
        let mut rgb = vec![[245; 3]; 1920 * 1080];
        let mut row = vec![[245; 3]; 1920 * 40];
        for x in (12..1800).step_by(180) {
            rgb_text(&mut row, 1920, x, 10, "TEXT", [90; 3]);
            rgb_text(&mut row, 1920, x + 48, 10, "LINK", [30, 90, 210]);
            rgb_text(&mut row, 1920, x + 96, 10, "TEXT", [90; 3]);
            underline(&mut row, 1920, x as usize + 48, x as usize + 94, 26, [30, 90, 210]);
        }
        for chunk in rgb.chunks_exact_mut(row.len()) {
            chunk.copy_from_slice(&row);
        }
        for (color_links, underline_links) in [(false, false), (false, true), (true, false), (true, true)] {
            let start = std::time::Instant::now();
            let detection = detect_rgb_with_trace(&rgb, (1920, 1080), (1920, 1080),
                DetectionLimits::default(), color_links, underline_links);
            eprintln!("1920x1080 color_links={color_links} underline_links={underline_links}: {:?}, {} targets, {} underline targets, {} retained mask bytes, {} metadata bytes",
                start.elapsed(), detection.regions.len(), accepted_underlines(&detection).len(),
                detection.trace.edges.len() + detection.trace.color_strokes.len(),
                detection.trace.components.capacity() * std::mem::size_of::<DiagnosticComponent>());
            assert_eq!(accepted_underlines(&detection).len(),
                if underline_links && !color_links { 270 } else { 0 });
            assert_eq!(detection.regions.len(), if color_links || underline_links { 540 } else { 270 });
            assert_trace_consistent(&detection);
        }
        let mut state = 17u32;
        for pixel in &mut rgb {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *pixel = [(state >> 24) as u8; 3];
        }
        let mut detection = Detection { regions: Vec::new(), trace: DetectionTrace {
            width: 1920, height: 1080, ..DetectionTrace::default()
        } };
        let gray = luma(&rgb);
        let start = std::time::Instant::now();
        add_underline_regions(&mut detection, &gray, DetectionLimits::default());
        eprintln!("1920x1080 noise underline-only work: {:?}, {} candidates, {} targets",
            start.elapsed(), detection.trace.components.len(), detection.regions.len());
        assert!(detection.regions.is_empty());
    }

    #[test]
    fn diagnostic_outcomes_follow_filter_order_and_deduplication() {
        let component = |x, y, width, height, edge_pixels| Component {
            bounds: Rect { x, y, width, height }, edge_pixels,
        };
        let (regions, trace) = classify_components(vec![
            component(10, 10, 30, 30, 100),
            component(20, 20, 10, 10, 30),
            component(10, 10, 30, 30, 100),
            component(100, 10, 2, 2, 7),
            component(110, 10, 7, 10, 20),
            component(150, 10, 500, 20, 1000),
        ], DetectionLimits::default());
        assert_eq!(trace.iter().map(|c| c.outcome).collect::<Vec<_>>(), [
            Outcome::Accepted, Outcome::NestedOrDuplicate, Outcome::NestedOrDuplicate,
            Outcome::InsufficientEdges, Outcome::TooSmall, Outcome::TooLarge,
        ]);
        assert_eq!(regions, [trace[0].bounds]);
    }

    #[test]
    fn snapshot_edges_components_and_targets_are_consistent() {
        let mut pixels = image(200, 120);
        stroke_rect(&mut pixels, 200, 10, 10, 30, 20);
        stroke_rect(&mut pixels, 200, 70, 10, 50, 70);
        let detection = detect_with_trace(&pixels, 200, 120, 1.0, DetectionLimits::default());
        let trace = detection.trace;
        assert_eq!((trace.width, trace.height, trace.edges.len()), (200, 120, 24000));
        assert_eq!(trace.edges.iter().map(|v| *v as usize).sum::<usize>(),
            trace.components.iter().map(|c| c.edge_pixels).sum::<usize>());
        assert_eq!(trace.components.len(), 2);
        assert_eq!(detection.regions.len(), 1);
        assert_eq!(trace.components.iter().filter(|c| c.outcome == Outcome::Accepted)
            .map(|c| c.bounds).collect::<Vec<_>>(), detection.regions);
        assert!(trace.components.iter().any(|c| c.outcome == Outcome::TooLarge));
        let blank = detect_with_trace(&image(20, 20), 20, 20, 1.0, DetectionLimits::default());
        assert!(blank.regions.is_empty());
        assert!(blank.trace.components.is_empty());
        assert_eq!(blank.trace.edges, vec![0; 400]);
    }

    #[test]
    #[ignore = "synthetic activation timing; run in release mode with nocapture"]
    fn diagnostic_snapshot_cost() {
        let mut pixels = image(1920, 1080);
        for y in (10..1000).step_by(40) {
            for x in (10..1800).step_by(100) {
                stroke_rect(&mut pixels, 1920, x, y, 70, 20);
            }
        }
        let start = std::time::Instant::now();
        let detection = detect_with_trace(&pixels, 1920, 1080, 1.0, DetectionLimits::default());
        eprintln!("1920x1080 diagnostic detection {:?}; {} targets; retained mask {} bytes; component metadata {} bytes",
            start.elapsed(), detection.regions.len(), detection.trace.edges.len(),
            detection.trace.components.capacity() * std::mem::size_of::<DiagnosticComponent>());
        assert_eq!(detection.regions.len(), 450);
    }

    fn image(width: usize, height: usize) -> Vec<u8> {
        vec![255; width * height]
    }

    fn stroke_rect(
        image: &mut [u8],
        width: usize,
        x: usize,
        y: usize,
        rect_w: usize,
        rect_h: usize,
    ) {
        for xx in x..x + rect_w {
            image[y * width + xx] = 0;
            image[(y + rect_h - 1) * width + xx] = 0;
        }
        for yy in y..y + rect_h {
            image[yy * width + x] = 0;
            image[yy * width + x + rect_w - 1] = 0;
        }
    }

    #[test]
    fn finds_simple_box_targets() {
        let mut pixels = image(96, 72);
        stroke_rect(&mut pixels, 96, 10, 8, 20, 12);
        stroke_rect(&mut pixels, 96, 42, 24, 28, 16);
        let rects = detect_regions_from_luma(&pixels, 96, 72, 1.0, DetectionLimits::default());
        assert_eq!(
            rects,
            vec![
                Rect {
                    x: 9,
                    y: 7,
                    width: 22,
                    height: 14
                },
                Rect {
                    x: 41,
                    y: 23,
                    width: 30,
                    height: 18
                },
            ]
        );
    }

    #[test]
    fn filters_background_nested_and_tiny_regions() {
        let mut pixels = image(120, 90);
        stroke_rect(&mut pixels, 120, 8, 8, 46, 34);
        stroke_rect(&mut pixels, 120, 20, 20, 12, 4);
        stroke_rect(&mut pixels, 120, 64, 10, 2, 2);
        stroke_rect(&mut pixels, 120, 74, 12, 30, 22);
        let rects = detect_regions_from_luma(&pixels, 120, 90, 1.0, DetectionLimits::default());
        assert_eq!(
            rects,
            vec![
                Rect {
                    x: 7,
                    y: 7,
                    width: 48,
                    height: 36
                },
                Rect {
                    x: 73,
                    y: 11,
                    width: 32,
                    height: 24
                }
            ]
        );
    }

    #[test]
    fn dilation_scales_with_output_scale() {
        let mut pixels = image(200, 120);
        stroke_rect(&mut pixels, 200, 20, 20, 32, 18);
        let rects = detect_regions_from_luma(&pixels, 200, 120, 2.0, DetectionLimits::default());
        assert_eq!(
            rects,
            vec![Rect {
                x: 18,
                y: 18,
                width: 36,
                height: 22
            }]
        );
    }

    fn text(pixels: &mut [u8], width: u32, height: u32, x: i32, y: i32, s: &str, color: u8) {
        use crate::font::{Canvas, Color};
        let mut rgba: Vec<u8> = pixels.iter().flat_map(|&v| [v, v, v, 255]).collect();
        Canvas::new(&mut rgba, width, height).draw_text(
            x,
            y,
            s,
            2,
            Color::rgba(color, color, color, 255),
        );
        for (p, rgba) in pixels.iter_mut().zip(rgba.chunks_exact(4)) {
            *p = rgba[0];
        }
    }

    fn targets(pixels: &[u8], width: u32, height: u32) -> Vec<Rect> {
        detect_regions_from_luma(pixels, width, height, 1.0, DetectionLimits::default())
    }

    fn covers(rect: &Rect, x: u32, y: u32) -> bool {
        rect.x <= x && rect.y <= y && rect.x + rect.width > x && rect.y + rect.height > y
    }

    #[test]
    fn groups_unenclosed_text_without_merging_separate_links_or_rows() {
        let mut pixels = image(260, 100);
        text(&mut pixels, 260, 100, 12, 10, "file", 30);
        text(&mut pixels, 260, 100, 100, 10, "edit", 30);
        text(&mut pixels, 260, 100, 12, 42, "view", 30);
        let rects = targets(&pixels, 260, 100);
        assert_eq!(rects.len(), 3, "{rects:?}");
        for (x, y) in [(35, 16), (120, 16), (35, 48)] {
            assert_eq!(rects.iter().filter(|r| covers(r, x, y)).count(), 1);
        }
    }

    #[test]
    fn finds_open_icons_and_filled_buttons_and_deduplicates_labels() {
        let mut pixels = image(280, 120);
        // A hamburger and an open chevron have no enclosed background.
        for y in [12, 17, 22] {
            for x in 12..32 {
                pixels[y * 280 + x] = 0;
            }
        }
        for i in 0..10 {
            pixels[(12 + i) * 280 + 70 + i] = 0;
            pixels[(22 + i) * 280 + 79 - i] = 0;
        }
        for y in 60..98 {
            for x in 12..100 {
                pixels[y * 280 + x] = 30;
            }
        }
        text(&mut pixels, 280, 120, 32, 72, "save", 240);
        stroke_rect(&mut pixels, 280, 142, 60, 88, 38);
        text(&mut pixels, 280, 120, 160, 72, "open", 20);
        let rects = targets(&pixels, 280, 120);
        assert_eq!(rects.len(), 4, "{rects:?}");
        for (x, y) in [(22, 17), (75, 22), (56, 79), (186, 79)] {
            assert_eq!(rects.iter().filter(|r| covers(r, x, y)).count(), 1);
        }
    }

    #[test]
    fn retains_light_and_dark_antialiased_text_but_rejects_noise() {
        for (background, foreground) in [(245, 190), (25, 90)] {
            let mut pixels = vec![background; 180 * 80];
            // Deterministic low-amplitude texture plus isolated high-contrast specks.
            for (i, p) in pixels.iter_mut().enumerate() {
                *p = (*p as i16 + ((i * 17 % 7) as i16 - 3)) as u8;
            }
            for (x, y) in [(140, 15), (150, 45), (130, 65)] {
                pixels[y * 180 + x] = 255 - background;
            }
            text(&mut pixels, 180, 80, 12, 20, "link", foreground);
            let rects = targets(&pixels, 180, 80);
            assert_eq!(rects.len(), 1, "{rects:?}");
            assert!(covers(&rects[0], 32, 26));
        }
        assert!(targets(&vec![127; 180 * 80], 180, 80).is_empty());
    }

    #[test]
    fn logical_targets_are_stable_across_fractional_and_integer_scales() {
        let mut pixels = image(240, 120);
        text(&mut pixels, 240, 120, 16, 12, "click", 30);
        stroke_rect(&mut pixels, 240, 120, 60, 80, 32);
        let expected = targets(&pixels, 240, 120);
        assert_eq!(expected.len(), 2);
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let width = (240.0 * scale) as u32;
            let height = (120.0 * scale) as u32;
            let scaled = resize_luma(&pixels, 240, 120, width, height);
            let actual =
                detect_regions_from_luma(&scaled, width, height, scale, DetectionLimits::default());
            assert_eq!(actual.len(), expected.len(), "scale={scale}: {actual:?}");
            for (actual, expected) in actual.iter().zip(&expected) {
                let actual = map_rect(*actual, width, height, 240, 120);
                assert!(actual.x.abs_diff(expected.x) <= 2);
                assert!(actual.y.abs_diff(expected.y) <= 2);
                assert!(actual.width.abs_diff(expected.width) <= 4);
                assert!(actual.height.abs_diff(expected.height) <= 4);
            }
        }
    }

    #[test]
    fn rejects_invalid_images_and_enforces_logical_limits() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                detect_regions_from_luma(&[0; 100], 10, 10, scale, DetectionLimits::default())
                    .is_empty()
            );
        }
        assert!(targets(&[], 10, 10).is_empty());
        assert!(targets(&[], 0, 0).is_empty());
        let mut pixels = image(200, 100);
        stroke_rect(&mut pixels, 200, 12, 12, 100, 60);
        assert!(targets(&pixels, 200, 100).is_empty());
        let limits = DetectionLimits {
            max_height: 80.0,
            ..DetectionLimits::default()
        };
        assert_eq!(
            detect_regions_from_luma(&pixels, 200, 100, 1.0, limits).len(),
            1
        );
    }

    #[test]
    fn large_panel_does_not_hide_text_and_duplicate_bounds_are_removed() {
        let mut pixels = image(240, 120);
        stroke_rect(&mut pixels, 240, 2, 2, 236, 116);
        text(&mut pixels, 240, 120, 16, 16, "file", 20);
        text(&mut pixels, 240, 120, 16, 42, "edit", 20);
        assert_eq!(targets(&pixels, 240, 120).len(), 2);
        let first = Rect {
            x: 10,
            y: 10,
            width: 40,
            height: 20,
        };
        let second = Rect { x: 100, ..first };
        assert_eq!(
            filter_nested(vec![first, second, first], 1.0),
            vec![first, second]
        );
    }
}
