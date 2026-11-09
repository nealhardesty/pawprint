//! What the settings window (SPEC §8.7) and the preview window are drawn
//! with and driven by, written once for every backend: window input as
//! the backends report it, and a small pixel canvas that draws in "units"
//! (one unit is one pixel of the bundled fonts and the mascot art) and is
//! scaled up by a whole number at the end, like the badge.

use pawprint_sprite::{Font, Rgb, RgbaImage};

use crate::geom::Point;

/// Input to a normal window, as each backend translates it. Positions are
/// physical pixels from the top-left of the window's content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// The pointer moved.
    Move(Point),
    /// The primary button went down.
    Press(Point),
    /// The primary button came up.
    Release(Point),
    /// The pointer left the window.
    Leave,
    /// Wheel or touchpad, in lines; positive scrolls toward the end.
    Scroll(f64),
    /// A key that edits or navigates.
    Key(UiKey),
    /// A typed character (never a control character).
    Text(char),
    /// A letter with Ctrl held (Command on macOS): `z` undoes, `w` and `q`
    /// close.
    Shortcut(char),
    /// The window's close button.
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    Enter,
    Escape,
    Tab,
}

/// The window's colors: dark slate, cream text, marigold for what is
/// chosen (the accent palette's `marigold`).
pub mod theme {
    use pawprint_sprite::Rgb;

    pub const BG: Rgb = Rgb(0x23, 0x26, 0x2b);
    pub const PANEL: Rgb = Rgb(0x2e, 0x32, 0x38);
    pub const RAISED: Rgb = Rgb(0x3b, 0x40, 0x48);
    pub const HOVER: Rgb = Rgb(0x4a, 0x50, 0x5a);
    pub const LINE: Rgb = Rgb(0x50, 0x57, 0x62);
    pub const FIELD: Rgb = Rgb(0x18, 0x1a, 0x1e);
    pub const TEXT: Rgb = Rgb(0xec, 0xe8, 0xdc);
    pub const DIM: Rgb = Rgb(0x93, 0x9a, 0xa4);
    pub const PICK: Rgb = Rgb(0xe8, 0x91, 0x2d);
    pub const ON_PICK: Rgb = Rgb(0x1b, 0x1b, 0x1f);
    pub const GOOD: Rgb = Rgb(0x8c, 0xc0, 0x6c);
    pub const BAD: Rgb = Rgb(0xec, 0x6a, 0x5e);
}

/// A rectangle in units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn inset(&self, by: i32) -> Rect {
        Rect::new(self.x + by, self.y + by, self.w - 2 * by, self.h - 2 * by)
    }

    fn intersect(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        Rect::new(x, y, (r - x).max(0), (b - y).max(0))
    }
}

/// Width in units of `text` in `font`.
pub fn text_width(font: &Font, text: &str) -> i32 {
    let n = text.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * font.metrics.advance as i32 - (font.metrics.advance - font.metrics.width) as i32
    }
}

/// An opaque image in units, with a clip rectangle.
pub struct Canvas {
    pub img: RgbaImage,
    clip: Rect,
}

impl Canvas {
    pub fn new(w: i32, h: i32, bg: Rgb) -> Canvas {
        let mut c = Canvas {
            img: RgbaImage::new(w as u32, h as u32),
            clip: Rect::new(0, 0, w, h),
        };
        c.fill(c.clip, bg);
        c
    }

    fn full(&self) -> Rect {
        Rect::new(0, 0, self.img.width as i32, self.img.height as i32)
    }

    /// Draw only inside `r` until [`Canvas::unclip`].
    pub fn clip(&mut self, r: Rect) {
        self.clip = r.intersect(&self.full());
    }

    pub fn unclip(&mut self) {
        self.clip = self.full();
    }

    fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if self.clip.contains(x, y) {
            let i = ((y as u32 * self.img.width + x as u32) * 4) as usize;
            self.img.data[i..i + 4].copy_from_slice(&[c.0, c.1, c.2, 255]);
        }
    }

    pub fn fill(&mut self, r: Rect, c: Rgb) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                self.put(x, y, c);
            }
        }
    }

    /// A one-unit outline just inside `r`.
    pub fn border(&mut self, r: Rect, c: Rgb) {
        self.fill(Rect::new(r.x, r.y, r.w, 1), c);
        self.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), c);
        self.fill(Rect::new(r.x, r.y, 1, r.h), c);
        self.fill(Rect::new(r.right() - 1, r.y, 1, r.h), c);
    }

    /// Draw `text` with its top-left at (x, y); returns the width drawn.
    pub fn text(&mut self, font: &Font, text: &str, x: i32, y: i32, c: Rgb) -> i32 {
        let m = font.metrics;
        for (i, ch) in text.chars().enumerate() {
            let gx = x + (i as u32 * m.advance) as i32;
            for gy in 0..m.height {
                for dx in 0..m.width {
                    if font.ink(ch, dx, gy) {
                        self.put(gx + dx as i32, y + gy as i32, c);
                    }
                }
            }
        }
        text_width(font, text)
    }

    /// Copy the opaque pixels of `img`, each `scale` × `scale` units.
    pub fn blit(&mut self, img: &RgbaImage, x: i32, y: i32, scale: i32) {
        for iy in 0..img.height {
            for ix in 0..img.width {
                let p = img.pixel(ix, iy);
                if p[3] != 0 {
                    self.fill(
                        Rect::new(x + ix as i32 * scale, y + iy as i32 * scale, scale, scale),
                        Rgb(p[0], p[1], p[2]),
                    );
                }
            }
        }
    }
}

/// Nearest-neighbor upscale by a whole number, a row at a time.
pub fn upscale(img: &RgbaImage, k: u32) -> RgbaImage {
    let k = k.max(1) as usize;
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = RgbaImage::new((w * k) as u32, (h * k) as u32);
    let stride = w * k * 4;
    for y in 0..h {
        let row = &mut out.data[y * k * stride..(y * k + 1) * stride];
        for x in 0..w {
            let px = &img.data[(y * w + x) * 4..(y * w + x) * 4 + 4];
            for i in 0..k {
                row[(x * k + i) * 4..(x * k + i) * 4 + 4].copy_from_slice(px);
            }
        }
        let (first, rest) = out.data[y * k * stride..(y + 1) * k * stride].split_at_mut(stride);
        for copy in rest.chunks_mut(stride) {
            copy.copy_from_slice(first);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::bundle;
    use crate::render::scale;

    #[test]
    fn upscale_matches_the_badge_scaler() {
        let mut img = RgbaImage::new(3, 2);
        img.put(0, 0, [1, 2, 3, 255]);
        img.put(2, 1, [9, 8, 7, 255]);
        for k in [1, 2, 3] {
            assert_eq!(upscale(&img, k), scale(&img, k), "k = {k}");
        }
    }

    #[test]
    fn text_is_measured_and_drawn_with_the_badge_font() {
        let font = &bundle().font;
        assert_eq!(text_width(font, ""), 0);
        assert_eq!(text_width(font, "A"), 5);
        assert_eq!(text_width(font, "AB"), 11);
        let mut c = Canvas::new(20, 9, theme::BG);
        assert_eq!(c.text(font, "I", 1, 1, theme::TEXT), 5);
        let ink = |c: &Canvas| {
            c.img
                .data
                .chunks(4)
                .filter(|p| p[..3] == [theme::TEXT.0, theme::TEXT.1, theme::TEXT.2])
                .count()
        };
        let want = (0..7)
            .map(|y| (0..5).filter(|&x| font.ink('I', x, y)).count())
            .sum::<usize>();
        assert_eq!(ink(&c), want);
        // Clipped away entirely.
        let mut d = Canvas::new(20, 9, theme::BG);
        d.clip(Rect::new(10, 0, 10, 9));
        d.text(font, "I", 1, 1, theme::TEXT);
        assert_eq!(ink(&d), 0);
    }

    #[test]
    fn rects() {
        let r = Rect::new(2, 3, 4, 5);
        assert!(r.contains(2, 3) && r.contains(5, 7) && !r.contains(6, 3));
        assert_eq!(r.inset(1), Rect::new(3, 4, 2, 3));
        assert_eq!(r.intersect(&Rect::new(0, 0, 4, 4)), Rect::new(2, 3, 2, 1));
    }
}
