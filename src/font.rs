#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let digits = text.strip_prefix('#').unwrap_or(text);
        let [r, g, b, a] = match digits.len() {
            6 => [
                u8::from_str_radix(&digits[0..2], 16).map_err(|_| invalid_color(text))?,
                u8::from_str_radix(&digits[2..4], 16).map_err(|_| invalid_color(text))?,
                u8::from_str_radix(&digits[4..6], 16).map_err(|_| invalid_color(text))?,
                0xff,
            ],
            8 => [
                u8::from_str_radix(&digits[0..2], 16).map_err(|_| invalid_color(text))?,
                u8::from_str_radix(&digits[2..4], 16).map_err(|_| invalid_color(text))?,
                u8::from_str_radix(&digits[4..6], 16).map_err(|_| invalid_color(text))?,
                u8::from_str_radix(&digits[6..8], 16).map_err(|_| invalid_color(text))?,
            ],
            _ => return Err(invalid_color(text)),
        };
        Ok(Self { r, g, b, a })
    }

    fn premultiplied(self) -> [u8; 4] {
        let scale = self.a as u16;
        [
            ((self.b as u16 * scale + 127) / 255) as u8,
            ((self.g as u16 * scale + 127) / 255) as u8,
            ((self.r as u16 * scale + 127) / 255) as u8,
            self.a,
        ]
    }
}

fn invalid_color(text: &str) -> String {
    format!("invalid color {text:?}; expected #RRGGBB or #RRGGBBAA")
}

pub struct Canvas<'a> {
    pixels: &'a mut [u8],
    width: usize,
    height: usize,
}

impl<'a> Canvas<'a> {
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32) -> Self {
        Self {
            pixels,
            width: width as usize,
            height: height as usize,
        }
    }

    pub fn clear(&mut self, color: Color) {
        let pixel = color.premultiplied();
        for chunk in self.pixels.chunks_exact_mut(4) {
            chunk.copy_from_slice(&pixel);
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        self.rect_impl(x, y, width, height, color, true);
    }

    pub fn stroke_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        if width <= 0 || height <= 0 {
            return;
        }
        self.fill_rect(x, y, width, 1, color);
        self.fill_rect(x, y + height - 1, width, 1, color);
        self.fill_rect(x, y, 1, height, color);
        self.fill_rect(x + width - 1, y, 1, height, color);
    }

    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, scale: i32, color: Color) {
        if scale <= 0 {
            return;
        }
        for (index, ch) in text.chars().enumerate() {
            if let Some(rows) = glyph_rows(ch) {
                for (row, bits) in rows.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) == 0 {
                            continue;
                        }
                        self.fill_rect(
                            x + (index as i32 * 6 + col) * scale,
                            y + row as i32 * scale,
                            scale,
                            scale,
                            color,
                        );
                    }
                }
            }
        }
    }

    pub fn text_size(text: &str, scale: i32) -> (i32, i32) {
        let count = text.chars().count() as i32;
        if count == 0 {
            (0, 0)
        } else {
            ((count * 6 - 1) * scale, 7 * scale)
        }
    }

    fn rect_impl(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color, fill: bool) {
        if width <= 0 || height <= 0 {
            return;
        }
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x + width).min(self.width as i32).max(0) as usize;
        let y1 = (y + height).min(self.height as i32).max(0) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let source = color.premultiplied();
        for yy in y0..y1 {
            for xx in x0..x1 {
                let index = (yy * self.width + xx) * 4;
                blend(&mut self.pixels[index..index + 4], source);
            }
        }
        if !fill {
            unreachable!();
        }
    }
}

fn blend(dst: &mut [u8], src: [u8; 4]) {
    let alpha = src[3] as u16;
    let inv = 255 - alpha;
    dst[0] = (src[0] as u16 + (dst[0] as u16 * inv + 127) / 255) as u8;
    dst[1] = (src[1] as u16 + (dst[1] as u16 * inv + 127) / 255) as u8;
    dst[2] = (src[2] as u16 + (dst[2] as u16 * inv + 127) / 255) as u8;
    dst[3] = (alpha + (dst[3] as u16 * inv + 127) / 255) as u8;
}

fn glyph_rows(ch: char) -> Option<[u8; 7]> {
    Some(match ch.to_ascii_lowercase() {
        'a' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'b' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'c' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'd' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'e' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'f' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'g' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'h' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'i' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'j' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'k' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'l' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'm' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'n' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'o' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'p' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'r' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        's' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        't' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'u' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'v' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100,
        ],
        'w' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'x' => [
            0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b01010, 0b10001,
        ],
        'y' => [
            0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'z' => [
            0b11111, 0b00010, 0b00100, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_rgba_hex_colors() {
        assert_eq!(
            Color::parse("#00ff00e0").unwrap(),
            Color::rgba(0, 255, 0, 0xe0)
        );
        assert_eq!(
            Color::parse("ffd54f").unwrap(),
            Color::rgba(0xff, 0xd5, 0x4f, 0xff)
        );
        for invalid in ["#0", "#12345", "#gg0000", "blue"] {
            assert!(Color::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn draws_translucent_text_and_boxes() {
        let mut pixels = vec![0; 20 * 20 * 4];
        let mut canvas = Canvas::new(&mut pixels, 20, 20);
        canvas.clear(Color::rgba(0, 0, 0, 0));
        canvas.fill_rect(1, 1, 10, 10, Color::rgba(0, 255, 0, 0x20));
        canvas.stroke_rect(1, 1, 10, 10, Color::rgba(255, 255, 255, 0xff));
        canvas.draw_text(2, 2, "ab", 1, Color::rgba(255, 128, 0, 0xff));
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert_eq!(Canvas::text_size("ab", 2), (22, 14));
    }
}
