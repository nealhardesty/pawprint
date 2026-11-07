//! The compositor, integer scaler, and the one pixel format every backend
//! receives (SPEC §4.5, §10.6).

use pawprint_sprite::{
    CrateArt, CrateColor, Font, IndexedLayer, Mascot, Ramp, Rgb, RgbaImage, Shade, ramp,
};

use crate::geom::Point;
use crate::layout::{self, Anchor, BadgeLayout, LabelSize, PlateMetrics};

/// Label text color, fixed for legibility (SPEC §4.4).
pub const TEXT_COLOR: Rgb = Rgb(0x1b, 0x1b, 0x1f);

/// A frame ready for any backend: premultiplied little-endian ARGB8888
/// (bytes B, G, R, A per pixel, rows top to bottom, no padding) plus a 1-bit
/// opacity mask (rows padded to whole bytes, least significant bit first),
/// which feeds the X11 shape fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub mask: Vec<u8>,
    pub mask_stride: usize,
}

impl Frame {
    pub fn from_rgba(img: &RgbaImage) -> Frame {
        let mask_stride = img.width.div_ceil(8) as usize;
        let mut mask = vec![0u8; mask_stride * img.height as usize];
        let mut bgra = Vec::with_capacity(img.data.len());
        for (i, px) in img.data.as_chunks::<4>().0.iter().enumerate() {
            let [r, g, b, a] = [px[0], px[1], px[2], px[3]].map(u16::from);
            let pre = |c: u16| ((c * a + 127) / 255) as u8;
            bgra.extend_from_slice(&[pre(b), pre(g), pre(r), a as u8]);
            if a != 0 {
                let (x, y) = (i % img.width as usize, i / img.width as usize);
                mask[y * mask_stride + x / 8] |= 1 << (x % 8);
            }
        }
        Frame {
            width: img.width,
            height: img.height,
            bgra,
            mask,
            mask_stride,
        }
    }

    pub fn opaque(&self, x: u32, y: u32) -> bool {
        self.mask[y as usize * self.mask_stride + x as usize / 8] & (1 << (x % 8)) != 0
    }
}

/// Nearest-neighbor upscale by an integer factor.
pub fn scale(img: &RgbaImage, factor: u32) -> RgbaImage {
    let factor = factor.max(1);
    let mut out = RgbaImage::new(img.width * factor, img.height * factor);
    for y in 0..out.height {
        for x in 0..out.width {
            out.put(
                i64::from(x),
                i64::from(y),
                img.pixel(x / factor, y / factor),
            );
        }
    }
    out
}

/// A tray or menu-bar icon (SPEC §11.1.5, §11.2): `img` (the 32 × 32 rest
/// frame) drawn nearest-neighbor into a `size` × `size` square with a
/// transparent `margin` on every side. Nearest neighbor keeps the binary
/// alpha and the pixel-art edges at any size.
pub fn tray_icon(img: &RgbaImage, size: u32, margin: u32) -> RgbaImage {
    let mut out = RgbaImage::new(size, size);
    let inner = size.saturating_sub(2 * margin).max(1);
    for y in 0..inner {
        for x in 0..inner {
            let p = img.pixel(x * img.width / inner, y * img.height / inner);
            out.put(i64::from(x + margin), i64::from(y + margin), p);
        }
    }
    out
}

/// Copy the opaque pixels of `src` onto `dst` with `src`'s top-left at `at`.
fn blit(dst: &mut RgbaImage, src: &RgbaImage, at: Point) {
    for y in 0..src.height {
        for x in 0..src.width {
            let px = src.pixel(x, y);
            if px[3] != 0 {
                dst.put(
                    i64::from(at.x) + i64::from(x),
                    i64::from(at.y) + i64::from(y),
                    px,
                );
            }
        }
    }
}

fn crate_color(c: CrateColor, r: &Ramp) -> Rgb {
    match c {
        CrateColor::Fixed(rgb) => rgb,
        CrateColor::Accent(Shade::Shadow) => r.shadow,
        CrateColor::Accent(Shade::Base) => r.base,
        CrateColor::Accent(Shade::Highlight) => r.highlight,
    }
}

/// Draw an indexed crate layer at its own position plus `offset`, with each
/// layer pixel covering `unit` × `unit` image pixels.
fn draw_layer(
    dst: &mut RgbaImage,
    layer: &IndexedLayer,
    art: &CrateArt,
    accent: Rgb,
    offset: Point,
    unit: u32,
) {
    // The ramp once per layer, not per pixel.
    let r = ramp(accent);
    let colors: Vec<Rgb> = art.colors.iter().map(|&c| crate_color(c, &r)).collect();
    for (i, &p) in layer.pixels.iter().enumerate() {
        if p == 0 {
            continue;
        }
        let c = colors[usize::from(p) - 1];
        let lx = layer.x + (i as u32 % layer.width) as i32;
        let ly = layer.y + (i as u32 / layer.width) as i32;
        fill(
            dst,
            offset.x + lx * unit as i32,
            offset.y + ly * unit as i32,
            unit,
            unit,
            [c.0, c.1, c.2, 255],
        );
    }
}

fn fill(dst: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, px: [u8; 4]) {
    for dy in 0..h {
        for dx in 0..w {
            dst.put(
                i64::from(x) + i64::from(dx),
                i64::from(y) + i64::from(dy),
                px,
            );
        }
    }
}

/// Crate back, mascot frame, crate front, at 1× (32 × 32).
pub fn compose_1x(art: &CrateArt, mascot: &Mascot, frame: usize, accent: Rgb) -> RgbaImage {
    let mut img = RgbaImage::new(32, 32);
    draw_layer(&mut img, &art.back, art, accent, Point::default(), 1);
    blit(&mut img, &mascot.frame_rgba(frame), Point::default());
    draw_layer(&mut img, &art.front, art, accent, Point::default(), 1);
    img
}

pub fn plate_metrics(art: &CrateArt) -> PlateMetrics {
    PlateMetrics {
        left: art.plate_left.width,
        right: art.plate_right.width,
        height: art.plate_mid.height,
    }
}

/// What to draw.
#[derive(Debug, Clone, Copy)]
pub struct BadgeSpec<'a> {
    pub mascot: &'a Mascot,
    pub frame: usize,
    pub label: Option<&'a str>,
    pub accent: Rgb,
    pub size: u32,
    pub label_size: LabelSize,
    /// Decides which side a size-32 nameplate goes on.
    pub anchor: Anchor,
    pub display_scale: f64,
}

pub fn layout_for(spec: &BadgeSpec<'_>, art: &CrateArt) -> BadgeLayout {
    layout::badge_layout(
        spec.size,
        spec.label.map(|l| l.chars().count()),
        spec.label_size,
        spec.anchor,
        spec.display_scale,
        plate_metrics(art),
    )
}

/// Render the whole badge at physical resolution.
pub fn render_badge(spec: &BadgeSpec<'_>, art: &CrateArt, font: &Font) -> RgbaImage {
    let layout = layout_for(spec, art);
    let mut img = RgbaImage::new(layout.width, layout.height);
    let body = scale(
        &compose_1x(art, spec.mascot, spec.frame, spec.accent),
        layout.art_scale,
    );
    blit(&mut img, &body, layout.crate_origin);

    if let (Some(text), Some(label)) = (spec.label, layout.label) {
        let f = layout.font_scale;
        if let Some(plate) = label.plate {
            let origin = Point::new(plate.x, plate.y);
            draw_layer(&mut img, &art.plate_left, art, spec.accent, origin, f);
            for col in 0..label.plate_columns {
                let at = Point::new(
                    origin.x + ((art.plate_left.width + col) * f) as i32,
                    origin.y,
                );
                draw_layer(&mut img, &art.plate_mid, art, spec.accent, at, f);
            }
            let right = Point::new(
                origin.x + ((art.plate_left.width + label.plate_columns) * f) as i32,
                origin.y,
            );
            draw_layer(&mut img, &art.plate_right, art, spec.accent, right, f);
        }
        draw_text(&mut img, font, text, label.text, f);
    }
    img
}

fn draw_text(img: &mut RgbaImage, font: &Font, text: &str, at: Point, f: u32) {
    let ink = [TEXT_COLOR.0, TEXT_COLOR.1, TEXT_COLOR.2, 255];
    for (i, c) in text.chars().enumerate() {
        let gx = at.x + (i as u32 * Font::ADVANCE * f) as i32;
        for y in 0..Font::HEIGHT {
            for x in 0..Font::WIDTH {
                if font.ink(c, x, y) {
                    fill(img, gx + (x * f) as i32, at.y + (y * f) as i32, f, f, ink);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, bundle};

    fn owl_spec(label: Option<&str>, size: u32) -> BadgeSpec<'_> {
        BadgeSpec {
            mascot: Catalog::bundled().get("owl").unwrap(),
            frame: 0,
            label,
            accent: Rgb(0x7b, 0x5e, 0xa7),
            size,
            label_size: LabelSize::Auto,
            anchor: Anchor::TopRight,
            display_scale: 1.0,
        }
    }

    #[test]
    fn frame_format_is_premultiplied_bgra_with_lsb_mask() {
        let mut img = RgbaImage::new(9, 2);
        img.put(0, 0, [10, 20, 30, 255]);
        img.put(8, 1, [200, 100, 50, 255]);
        let f = Frame::from_rgba(&img);
        assert_eq!(&f.bgra[0..4], &[30, 20, 10, 255]);
        assert_eq!(&f.bgra[4..8], &[0, 0, 0, 0]);
        assert_eq!(f.mask_stride, 2);
        assert_eq!(f.mask, vec![0b0000_0001, 0, 0, 0b0000_0001]);
        assert!(f.opaque(0, 0) && f.opaque(8, 1) && !f.opaque(1, 0));
    }

    #[test]
    fn premultiplies_partial_alpha() {
        let mut img = RgbaImage::new(1, 1);
        img.put(0, 0, [255, 128, 0, 128]);
        assert_eq!(Frame::from_rgba(&img).bgra, vec![0, 64, 128, 128]);
    }

    #[test]
    fn integer_scaling_is_nearest_neighbor() {
        let mut img = RgbaImage::new(2, 1);
        img.put(1, 0, [1, 2, 3, 255]);
        let s = scale(&img, 3);
        assert_eq!((s.width, s.height), (6, 3));
        assert_eq!(s.pixel(2, 2), [0, 0, 0, 0]);
        assert_eq!(s.pixel(3, 0), [1, 2, 3, 255]);
        assert_eq!(s.pixel(5, 2), [1, 2, 3, 255]);
    }

    #[test]
    fn badge_has_only_binary_alpha_and_expected_size() {
        for size in [32, 64, 128] {
            let img = render_badge(
                &owl_spec(Some("OWL"), size),
                &bundle().crate_art,
                &bundle().font,
            );
            assert_eq!((img.width, img.height), (size, size));
            assert!(img.data.chunks(4).all(|p| p[3] == 0 || p[3] == 255));
        }
    }

    #[test]
    fn label_text_is_drawn_on_the_strip() {
        let b = bundle();
        let img = render_badge(&owl_spec(Some("I"), 64), &b.crate_art, &b.font);
        let text = [TEXT_COLOR.0, TEXT_COLOR.1, TEXT_COLOR.2, 255];
        let count = |img: &RgbaImage| img.data.chunks(4).filter(|p| *p == text).count();
        let without = render_badge(&owl_spec(None, 64), &b.crate_art, &b.font);
        // Ink pixels of 'I' at font scale 2 → 4 physical pixels each.
        let ink: u32 = (0..7)
            .map(|y| (0..5).filter(|&x| b.font.ink('I', x, y)).count() as u32)
            .sum();
        assert_eq!(count(&img) - count(&without), (ink * 4) as usize);
    }

    #[test]
    fn crate_is_tinted_by_accent() {
        let b = bundle();
        let a = compose_1x(
            &b.crate_art,
            Catalog::bundled().get("owl").unwrap(),
            0,
            Rgb(255, 0, 0),
        );
        let c = compose_1x(
            &b.crate_art,
            Catalog::bundled().get("owl").unwrap(),
            0,
            Rgb(0, 0, 255),
        );
        assert_ne!(a, c);
        // Strip pixels are fixed cream regardless of accent.
        assert_eq!(a.pixel(15, 25), [0xf2, 0xef, 0xe6, 255]);
        assert_eq!(a.pixel(15, 25), c.pixel(15, 25));
    }

    #[test]
    fn nameplate_badge_is_wider() {
        let b = bundle();
        let img = render_badge(&owl_spec(Some("PROD-OWL-02"), 64), &b.crate_art, &b.font);
        assert_eq!((img.width, img.height), (142, 90));
    }

    #[test]
    fn tray_icons_are_nearest_neighbor_with_binary_alpha() {
        let mut art = RgbaImage::new(32, 32);
        art.put(0, 0, [255, 0, 0, 255]);
        art.put(1, 0, [0, 255, 0, 255]);
        art.put(31, 31, [0, 0, 255, 255]);
        // 1:1, and doubled: every art pixel kept.
        assert_eq!(tray_icon(&art, 32, 0), art);
        assert_eq!(tray_icon(&art, 64, 0), scale(&art, 2));
        // Halved (a 16-px icon): every other art pixel.
        let half = tray_icon(&art, 16, 0);
        assert_eq!((half.width, half.height), (16, 16));
        assert_eq!(half.pixel(0, 0), [255, 0, 0, 255]);
        assert_eq!(half.pixel(15, 15), [0, 0, 0, 0]);
        // The macOS menu bar's 18-point box with a 1-point margin, at 2x.
        let boxed = tray_icon(&art, 36, 2);
        assert_eq!(boxed.pixel(1, 1), [0, 0, 0, 0], "margin");
        assert_eq!(boxed.pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(boxed.pixel(3, 2), [0, 255, 0, 255]);
        assert_eq!(boxed.pixel(33, 33), [0, 0, 255, 255]);
        assert_eq!(boxed.pixel(34, 34), [0, 0, 0, 0], "margin");
        // An odd size (20 px at 125%) keeps binary alpha.
        let odd = tray_icon(&art, 20, 0);
        assert!(odd.data.chunks(4).all(|p| p[3] == 0 || p[3] == 255));
    }
}
