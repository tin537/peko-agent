//! Pure-Rust 2D renderer for Lane A status overlays.
//!
//! Produces an RgbaBuffer the agent can:
//!   - return as the result of a "draw" tool
//!   - save to disk as PNG (via the `image` crate at the tool layer)
//!   - blit to /dev/graphics/fb0 in Lane A (no SurfaceFlinger needed)
//!
//! Capabilities: filled rectangles, axis-aligned lines, single-pixel
//! borders, text via the embedded 5x7 font in `font.rs`, fill operations.
//! Anti-aliasing is intentionally absent — the agent's status overlays
//! are sharp text, and AA would force fractional rasterisation we'd
//! have to invent without bringing in 100KB of dependencies.
//!
//! All operations are pure RgbaBuffer manipulation; the optional
//! `blit` module talks to peko-hal::FramebufferDevice for actual
//! display output and is only used in Lane A.

pub mod font;
pub mod blit;
pub use blit::{blit_to_framebuffer, read_fb_info, BlitError, BlitFormat, FbInfo};

use peko_hal::RgbaBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const BLACK: Rgba = Rgba(0, 0, 0, 255);
    pub const WHITE: Rgba = Rgba(255, 255, 255, 255);
    pub const RED: Rgba = Rgba(255, 0, 0, 255);
    pub const GREEN: Rgba = Rgba(0, 255, 0, 255);
    pub const BLUE: Rgba = Rgba(0, 0, 255, 255);
    pub const TRANSPARENT: Rgba = Rgba(0, 0, 0, 0);

    /// Parse `#RGB`, `#RRGGBB`, `#RRGGBBAA` style hex.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('#');
        match s.len() {
            3 => {
                let r = u8::from_str_radix(&s[0..1], 16).ok()? * 0x11;
                let g = u8::from_str_radix(&s[1..2], 16).ok()? * 0x11;
                let b = u8::from_str_radix(&s[2..3], 16).ok()? * 0x11;
                Some(Rgba(r, g, b, 255))
            }
            6 => Some(Rgba(
                u8::from_str_radix(&s[0..2], 16).ok()?,
                u8::from_str_radix(&s[2..4], 16).ok()?,
                u8::from_str_radix(&s[4..6], 16).ok()?,
                255,
            )),
            8 => Some(Rgba(
                u8::from_str_radix(&s[0..2], 16).ok()?,
                u8::from_str_radix(&s[2..4], 16).ok()?,
                u8::from_str_radix(&s[4..6], 16).ok()?,
                u8::from_str_radix(&s[6..8], 16).ok()?,
            )),
            _ => None,
        }
    }
}

pub struct Canvas {
    buf: RgbaBuffer,
}

impl Canvas {
    pub fn new(width: u32, height: u32, fill: Rgba) -> Self {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            data.extend_from_slice(&[fill.0, fill.1, fill.2, fill.3]);
        }
        Canvas {
            buf: RgbaBuffer { data, width, height },
        }
    }

    pub fn from_buffer(buf: RgbaBuffer) -> Self {
        Self { buf }
    }

    pub fn width(&self) -> u32 { self.buf.width }
    pub fn height(&self) -> u32 { self.buf.height }
    pub fn into_buffer(self) -> RgbaBuffer { self.buf }
    pub fn buffer(&self) -> &RgbaBuffer { &self.buf }

    pub fn put_pixel(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 {
            return;
        }
        if x as u32 >= self.buf.width || y as u32 >= self.buf.height {
            return;
        }
        let idx = ((y as u32) * self.buf.width + (x as u32)) as usize * 4;
        if idx + 4 > self.buf.data.len() {
            return;
        }
        // Alpha blend src over existing.
        let dst = &mut self.buf.data[idx..idx + 4];
        if color.3 == 255 {
            dst[0] = color.0;
            dst[1] = color.1;
            dst[2] = color.2;
            dst[3] = color.3;
        } else {
            let a = color.3 as u32;
            let inv = 255 - a;
            dst[0] = ((color.0 as u32 * a + dst[0] as u32 * inv) / 255) as u8;
            dst[1] = ((color.1 as u32 * a + dst[1] as u32 * inv) / 255) as u8;
            dst[2] = ((color.2 as u32 * a + dst[2] as u32 * inv) / 255) as u8;
            dst[3] = (dst[3] as u32 + a).min(255) as u8;
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgba) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.put_pixel(x + dx, y + dy, color);
            }
        }
    }

    pub fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Rgba) {
        if w == 0 || h == 0 {
            return;
        }
        let x2 = x + w as i32 - 1;
        let y2 = y + h as i32 - 1;
        for dx in 0..w as i32 {
            self.put_pixel(x + dx, y, color);
            self.put_pixel(x + dx, y2, color);
        }
        for dy in 0..h as i32 {
            self.put_pixel(x, y + dy, color);
            self.put_pixel(x2, y + dy, color);
        }
    }

    pub fn line_h(&mut self, x: i32, y: i32, w: u32, color: Rgba) {
        for dx in 0..w as i32 {
            self.put_pixel(x + dx, y, color);
        }
    }

    pub fn line_v(&mut self, x: i32, y: i32, h: u32, color: Rgba) {
        for dy in 0..h as i32 {
            self.put_pixel(x, y + dy, color);
        }
    }

    /// Render a single line of text. `scale` >= 1; columns × scale,
    /// rows × scale. Returns the cursor x after the last glyph.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: Rgba, scale: u32) -> i32 {
        let scale = scale.max(1) as i32;
        let mut cx = x;
        for c in text.chars() {
            let glyph = match font::glyph(c) {
                Some(g) => g,
                None => {
                    // Unknown chars become a 1-row line so the agent
                    // can see "I tried but couldn't render this".
                    self.fill_rect(
                        cx,
                        y + (font::GLYPH_H as i32 - 1) * scale,
                        font::GLYPH_W as u32 * scale as u32,
                        scale as u32,
                        color,
                    );
                    cx += (font::GLYPH_W as i32 + 1) * scale;
                    continue;
                }
            };
            for col in 0..font::GLYPH_W {
                let bits = glyph[col];
                for row in 0..font::GLYPH_H {
                    if (bits >> row) & 1 == 1 {
                        self.fill_rect(
                            cx + col as i32 * scale,
                            y + row as i32 * scale,
                            scale as u32,
                            scale as u32,
                            color,
                        );
                    }
                }
            }
            // 1-pixel gap between glyphs at scale 1.
            cx += (font::GLYPH_W as i32 + 1) * scale;
        }
        cx
    }

    /// Word-wrap `text` to fit `max_width_px` and render.
    pub fn draw_text_wrapped(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        max_width_px: u32,
        color: Rgba,
        scale: u32,
    ) -> i32 {
        let glyph_total_px = (font::GLYPH_W as u32 + 1) * scale.max(1);
        let line_height_px = (font::GLYPH_H as u32 + 1) * scale.max(1);
        let max_chars = (max_width_px / glyph_total_px).max(1) as usize;

        let mut cy = y;
        let mut current_line = String::new();
        for word in text.split_whitespace() {
            let candidate = if current_line.is_empty() {
                word.to_string()
            } else {
                format!("{current_line} {word}")
            };
            if candidate.chars().count() <= max_chars {
                current_line = candidate;
            } else {
                if !current_line.is_empty() {
                    self.draw_text(x, cy, &current_line, color, scale);
                    cy += line_height_px as i32;
                }
                current_line = word.to_string();
            }
        }
        if !current_line.is_empty() {
            self.draw_text(x, cy, &current_line, color, scale);
            cy += line_height_px as i32;
        }
        cy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_hex_parses_short_form() {
        assert_eq!(Rgba::from_hex("#fff"), Some(Rgba(255, 255, 255, 255)));
        assert_eq!(Rgba::from_hex("#000"), Some(Rgba(0, 0, 0, 255)));
        assert_eq!(Rgba::from_hex("#f00"), Some(Rgba(255, 0, 0, 255)));
    }

    #[test]
    fn rgba_hex_parses_long_form() {
        assert_eq!(Rgba::from_hex("#ff8800"), Some(Rgba(255, 136, 0, 255)));
        assert_eq!(Rgba::from_hex("#000000ff"), Some(Rgba(0, 0, 0, 255)));
        assert_eq!(Rgba::from_hex("00112233"), Some(Rgba(0, 17, 34, 51)));
    }

    #[test]
    fn rgba_hex_rejects_bad_input() {
        assert!(Rgba::from_hex("#zzz").is_none());
        assert!(Rgba::from_hex("#1234").is_none());
        assert!(Rgba::from_hex("").is_none());
    }

    #[test]
    fn canvas_new_fills_color() {
        let c = Canvas::new(2, 2, Rgba(10, 20, 30, 255));
        assert_eq!(c.buf.data.len(), 16);
        assert_eq!(&c.buf.data[0..4], &[10, 20, 30, 255]);
        assert_eq!(&c.buf.data[12..16], &[10, 20, 30, 255]);
    }

    #[test]
    fn put_pixel_clips_off_canvas() {
        let mut c = Canvas::new(2, 2, Rgba::BLACK);
        c.put_pixel(-1, 0, Rgba::WHITE);
        c.put_pixel(0, -1, Rgba::WHITE);
        c.put_pixel(2, 0, Rgba::WHITE);
        c.put_pixel(0, 2, Rgba::WHITE);
        assert_eq!(c.buf.data, vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn fill_rect_paints_correct_region() {
        let mut c = Canvas::new(4, 4, Rgba::BLACK);
        c.fill_rect(1, 1, 2, 2, Rgba::WHITE);
        // Top-left, top-right of inside region:
        let idx = |x: u32, y: u32| ((y * 4 + x) * 4) as usize;
        assert_eq!(&c.buf.data[idx(0, 0)..idx(0, 0) + 4], &[0, 0, 0, 255]);
        assert_eq!(&c.buf.data[idx(1, 1)..idx(1, 1) + 4], &[255, 255, 255, 255]);
        assert_eq!(&c.buf.data[idx(2, 2)..idx(2, 2) + 4], &[255, 255, 255, 255]);
        assert_eq!(&c.buf.data[idx(3, 3)..idx(3, 3) + 4], &[0, 0, 0, 255]);
    }

    #[test]
    fn alpha_blend_50pct() {
        let mut c = Canvas::new(1, 1, Rgba(0, 0, 0, 255));
        c.put_pixel(0, 0, Rgba(255, 255, 255, 128));
        // Result should be ~128, 128, 128
        let r = c.buf.data[0];
        assert!(r >= 126 && r <= 130);
    }

    #[test]
    fn draw_text_advances_cursor_by_glyph_width() {
        let mut c = Canvas::new(100, 20, Rgba::BLACK);
        let end = c.draw_text(0, 0, "AB", Rgba::WHITE, 1);
        // 2 glyphs * (5 + 1) = 12 px
        assert_eq!(end, 12);
    }

    #[test]
    fn draw_text_wrapped_breaks_at_max_width() {
        let mut c = Canvas::new(60, 60, Rgba::BLACK);
        // 60 px / 6 px per glyph = 10 chars per line.
        let final_y = c.draw_text_wrapped(0, 0, "HELLO WORLD AGENT", 60, Rgba::WHITE, 1);
        // Should advance at least 2 lines (text wraps).
        assert!(final_y >= (font::GLYPH_H as i32 + 1) * 2);
    }
}
