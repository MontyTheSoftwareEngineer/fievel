#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

pub fn detect_regions_from_luma(
    luma: &[u8],
    width: u32,
    height: u32,
    scale: f64,
    limits: DetectionLimits,
) -> Vec<Rect> {
    if width == 0
        || height == 0
        || !scale.is_finite()
        || scale <= 0.0
        || luma.len() < (width as usize).saturating_mul(height as usize)
    {
        return Vec::new();
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
    let rects: Vec<Rect> = components
        .into_iter()
        .filter(|component| component.edge_pixels >= 8)
        .map(|component| component.bounds)
        .filter(|rect| {
            let width = rect.width as f64;
            let height = rect.height as f64;
            width >= limits.min_width
                && width <= limits.max_width
                && height >= limits.min_height
                && height <= limits.max_height
        })
        .collect();
    let mut rects = filter_nested(rects, 1.0);
    rects.sort_by_key(|rect| (rect.y, rect.x, rect.width, rect.height));
    rects
        .into_iter()
        .map(|rect| map_rect(rect, logical_width, logical_height, width, height))
        .collect()
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
    if (width, height) == (out_w, out_h) {
        return luma.to_vec();
    }
    let mut out = vec![0; out_w as usize * out_h as usize];
    // Area sampling preserves thin antialiased strokes when reducing HiDPI captures.
    for y in 0..out_h {
        let top = y as f64 * height as f64 / out_h as f64;
        let bottom = (y + 1) as f64 * height as f64 / out_h as f64;
        for x in 0..out_w {
            let left = x as f64 * width as f64 / out_w as f64;
            let right = (x + 1) as f64 * width as f64 / out_w as f64;
            let mut sum = 0.0;
            for sy in top.floor() as u32..(bottom.ceil() as u32).min(height) {
                for sx in left.floor() as u32..(right.ceil() as u32).min(width) {
                    let weight = (bottom.min((sy + 1) as f64) - top.max(sy as f64))
                        * (right.min((sx + 1) as f64) - left.max(sx as f64));
                    sum += luma[(sy * width + sx) as usize] as f64 * weight;
                }
            }
            out[(y * out_w + x) as usize] = (sum / ((right - left) * (bottom - top))).round() as u8;
        }
    }
    out
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
