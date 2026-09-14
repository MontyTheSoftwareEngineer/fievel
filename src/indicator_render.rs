use evdev::KeyCode;
use crate::engine::SpeedMode;

const COLUMNS: usize = 40;
const CELL_WIDTH: u32 = 12;
const LINE_HEIGHT: u32 = 20;
const PADDING: u32 = 8;
const BACKGROUND: u32 = 0xc0181818;

fn foreground(speed: SpeedMode) -> u32 {
    match speed {
        SpeedMode::Normal => 0xf0f0f0f0,
        SpeedMode::Fast => 0xf000f000,
        SpeedMode::Slow => 0xf0f0f000,
    }
}

pub(super) fn key_label(key: KeyCode) -> String {
    let punctuation = match key {
        KeyCode::KEY_COMMA => Some(","),
        KeyCode::KEY_DOT => Some("."),
        KeyCode::KEY_SEMICOLON => Some(";"),
        KeyCode::KEY_APOSTROPHE => Some("'"),
        KeyCode::KEY_GRAVE => Some("`"),
        KeyCode::KEY_MINUS => Some("-"),
        KeyCode::KEY_EQUAL => Some("="),
        KeyCode::KEY_LEFTBRACE => Some("["),
        KeyCode::KEY_RIGHTBRACE => Some("]"),
        KeyCode::KEY_BACKSLASH => Some("\\"),
        KeyCode::KEY_SLASH => Some("/"),
        _ => None,
    };
    if let Some(label) = punctuation {
        return label.into();
    }
    let name = format!("{key:?}");
    name.strip_prefix("KEY_")
        .unwrap_or(&name)
        .to_ascii_uppercase()
}

pub(super) struct Layout {
    labels: Vec<String>,
    size: (u32, u32),
}

impl Layout {
    pub(super) fn new(keys: &[KeyCode]) -> Self {
        let labels: Vec<_> = keys.iter().map(|key| key_label(*key)).collect();
        let lines = wrap(&labels, COLUMNS);
        let columns = lines.iter().map(String::len).max().unwrap_or(1);
        let size = (
            columns as u32 * CELL_WIDTH + PADDING * 2,
            lines.len().max(1) as u32 * LINE_HEIGHT + PADDING * 2,
        );
        Self { labels, size }
    }

    pub(super) fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(super) fn pixels(&self, width: u32, height: u32, speed: SpeedMode) -> Vec<u8> {
        let mut pixels = vec![BACKGROUND; width as usize * height as usize];
        let foreground = foreground(speed);
        let columns = (width.saturating_sub(PADDING * 2) / CELL_WIDTH).max(1) as usize;
        for (line, text) in wrap(&self.labels, columns).iter().enumerate() {
            for (column, character) in text.chars().enumerate() {
                for (row, bits) in glyph(character).iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) == 0 {
                            continue;
                        }
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let x = PADDING + column as u32 * CELL_WIDTH + col * 2 + dx;
                                let y = PADDING + line as u32 * LINE_HEIGHT + row as u32 * 2 + dy;
                                if x < width && y < height {
                                    pixels[(y * width + x) as usize] = foreground;
                                }
                            }
                        }
                    }
                }
            }
        }
        pixels.into_iter().flat_map(u32::to_ne_bytes).collect()
    }
}

fn wrap(labels: &[String], columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for label in labels {
        if !line.is_empty() {
            if line.len() + 2 + label.len() <= columns {
                line.push_str("  ");
            } else {
                lines.push(std::mem::take(&mut line));
            }
        }
        for character in label.chars() {
            if line.len() == columns {
                lines.push(std::mem::take(&mut line));
            }
            line.push(character);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 25, 21, 19, 19, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' => [0; 7],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        ',' => [0, 0, 0, 0, 6, 4, 8],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        ';' => [0, 6, 6, 0, 6, 4, 8],
        ':' => [0, 6, 6, 0, 6, 6, 0],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '"' => [10, 10, 20, 0, 0, 0, 0],
        '`' => [8, 4, 2, 0, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '{' => [2, 4, 4, 8, 4, 4, 2],
        '}' => [8, 4, 4, 2, 4, 4, 8],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '@' => [14, 17, 23, 21, 23, 16, 14],
        '#' => [10, 10, 31, 10, 31, 10, 10],
        '$' => [4, 15, 20, 14, 5, 30, 4],
        '%' => [24, 25, 2, 4, 8, 19, 3],
        '^' => [4, 10, 17, 0, 0, 0, 0],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '*' => [0, 21, 14, 31, 14, 21, 0],
        '<' => [2, 4, 8, 16, 8, 4, 2],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '|' => [4; 7],
        '~' => [0, 0, 8, 21, 2, 0, 0],
        _ => [31, 17, 5, 4, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_labels_and_punctuation_are_readable() {
        assert_eq!(key_label(KeyCode::KEY_K), "K");
        assert_eq!(key_label(KeyCode::KEY_L), "L");
        assert_eq!(key_label(KeyCode::KEY_SPACE), "SPACE");
        assert_eq!(key_label(KeyCode::KEY_COMMA), ",");
        assert_eq!(key_label(KeyCode::KEY_DOT), ".");
        assert_eq!(key_label(KeyCode::KEY_LEFTCTRL), "LEFTCTRL");
        assert_eq!(key_label(KeyCode::KEY_BRIGHTNESSUP), "BRIGHTNESSUP");
        assert_ne!(glyph(','), glyph('.'));
        for character in '!'..='~' {
            if character.is_ascii_lowercase() {
                continue;
            }
            assert_ne!(glyph(character), glyph('\0'), "{character}");
            assert!(glyph(character).iter().any(|row| *row != 0));
        }
    }

    #[test]
    fn every_linux_keycode_label_has_supported_glyphs() {
        for code in 0..=0x2ff {
            let label = key_label(KeyCode::new(code));
            assert!(!label.is_empty());
            assert!(!label.starts_with("KEY_"), "{label}");
            for character in label.chars() {
                assert_ne!(glyph(character), glyph('\0'), "{label}: {character}");
            }
        }
    }

    #[test]
    fn history_wraps_with_bounded_width_and_preserves_order() {
        let layout = Layout::new(&[KeyCode::KEY_K, KeyCode::KEY_L, KeyCode::KEY_SPACE]);
        assert_eq!(wrap(&layout.labels, COLUMNS), ["K  L  SPACE"]);
        let layout = Layout::new(&[KeyCode::KEY_BRIGHTNESSUP; 32]);
        assert!(layout.size().0 <= COLUMNS as u32 * CELL_WIDTH + 2 * PADDING);
        assert!(layout.size().1 > LINE_HEIGHT + 2 * PADDING);
        let lines = wrap(&layout.labels, COLUMNS);
        assert_eq!(lines.join(" ").split_whitespace().count(), 32);
        assert!(lines.iter().all(|line| line.len() <= COLUMNS));
        assert_eq!(wrap(&["ABCDEFGHI".into()], 4), ["ABCD", "EFGH", "I"]);
    }

    #[test]
    fn speed_colors_are_white_green_yellow_and_premultiplied() {
        let layout = Layout::new(&[KeyCode::KEY_K, KeyCode::KEY_L]);
        let (width, height) = layout.size();
        for (speed, expected) in [
            (SpeedMode::Normal, 0xf0f0f0f0u32),
            (SpeedMode::Fast, 0xf000f000),
            (SpeedMode::Slow, 0xf0f0f000),
        ] {
            let pixels = layout.pixels(width, height, speed);
            assert!(pixels.chunks_exact(4).any(|pixel| pixel == expected.to_ne_bytes()));
            for pixel in pixels.chunks_exact(4) {
                let color = u32::from_ne_bytes(pixel.try_into().unwrap());
                assert!(color == expected || color == BACKGROUND);
                let alpha = color >> 24;
                for shift in [0, 8, 16] {
                    assert!(((color >> shift) & 255) <= alpha);
                }
            }
        }
    }

    #[test]
    fn dimensions_and_premultiplied_pixels_match_configuration() {
        let layout = Layout::new(&[KeyCode::KEY_K]);
        assert_eq!(layout.size(), (28, 36));
        let pixels = layout.pixels(28, 36, SpeedMode::Normal);
        assert_eq!(pixels.len(), 28 * 36 * 4);
        assert!(pixels
            .chunks_exact(4)
            .any(|pixel| pixel == foreground(SpeedMode::Normal).to_ne_bytes()));
        for pixel in pixels.chunks_exact(4) {
            let color = u32::from_ne_bytes(pixel.try_into().unwrap());
            assert!(color == foreground(SpeedMode::Normal) || color == BACKGROUND);
            let alpha = color >> 24;
            assert!((color & 255) <= alpha);
            assert!(((color >> 8) & 255) <= alpha);
            assert!(((color >> 16) & 255) <= alpha);
        }
        assert_eq!(layout.pixels(1, 1, SpeedMode::Normal).len(), 4);
        assert_eq!(layout.pixels(100, 50, SpeedMode::Normal).len(), 100 * 50 * 4);
    }
}
