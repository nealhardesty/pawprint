//! Colors: `#rrggbb` parsing and the accent ramp.

use serde::{Deserialize, Serialize};

/// An opaque sRGB color. Transparency is modeled separately (`Option<Rgb>` or
/// palette index 0), because every PAWPRINT pixel is fully opaque or fully
/// transparent (SPEC §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `#rrggbb` (case-insensitive). Anything else is `None`.
    pub fn parse_hex(s: &str) -> Option<Rgb> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// The three crate shades derived from one accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ramp {
    pub shadow: Rgb,
    pub base: Rgb,
    pub highlight: Rgb,
}

/// Lightness offset applied for the shadow and highlight shades.
const RAMP_LIGHTNESS_STEP: f64 = 0.12;

/// Expand an accent into its 3-step ramp by fixed HSL lightness offsets
/// (SPEC Appendix B). Used for bundled accents and for `#rrggbb` accents from
/// config alike, so both shade the same way.
pub fn ramp(base: Rgb) -> Ramp {
    let (h, s, l) = to_hsl(base);
    Ramp {
        shadow: from_hsl(h, s, (l - RAMP_LIGHTNESS_STEP).max(0.0)),
        base,
        highlight: from_hsl(h, s, (l + RAMP_LIGHTNESS_STEP).min(1.0)),
    }
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let [r, g, b] = [c.0, c.1, c.2].map(|v| f64::from(v) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to_u8 = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(to_u8(r), to_u8(g), to_u8(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_only_rrggbb() {
        let cases = [
            ("#1b1b1f", Some(Rgb(0x1b, 0x1b, 0x1f))),
            ("#FFFFFF", Some(Rgb(255, 255, 255))),
            ("1b1b1f", None),
            ("#fff", None),
            ("#gggggg", None),
            ("#1b1b1f0", None),
        ];
        for (input, want) in cases {
            assert_eq!(Rgb::parse_hex(input), want, "{input}");
        }
        assert_eq!(Rgb(0x7b, 0x5e, 0xa7).to_hex(), "#7b5ea7");
    }

    #[test]
    fn hsl_round_trips() {
        for c in [
            Rgb(0x7b, 0x5e, 0xa7),
            Rgb(0, 0, 0),
            Rgb(255, 255, 255),
            Rgb(0x2a, 0x8c, 0x87),
        ] {
            let (h, s, l) = to_hsl(c);
            assert_eq!(from_hsl(h, s, l), c);
        }
    }

    #[test]
    fn ramp_orders_lightness() {
        let r = ramp(Rgb(0x7b, 0x5e, 0xa7));
        let l = |c: Rgb| to_hsl(c).2;
        assert!(l(r.shadow) < l(r.base) && l(r.base) < l(r.highlight));
        assert_eq!(ramp(Rgb(0, 0, 0)).shadow, Rgb(0, 0, 0));
    }
}
