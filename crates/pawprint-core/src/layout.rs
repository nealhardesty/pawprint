//! Placement and badge geometry (SPEC §4.2–4.4, §6.1, §6.3): anchor/offset
//! ↔ rectangle, clamping, the 3 × 3 snap, and where the label goes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geom::{PhysicalRect, Point};
use pawprint_sprite::{FRAME_SIZE, Metrics};

/// One of the nine anchor points of the usable area.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    Top,
    #[default]
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

/// Position along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Start,
    Center,
    End,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top-left",
            Anchor::Top => "top",
            Anchor::TopRight => "top-right",
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
            Anchor::BottomLeft => "bottom-left",
            Anchor::Bottom => "bottom",
            Anchor::BottomRight => "bottom-right",
        }
    }

    pub fn parse(s: &str) -> Option<Anchor> {
        Anchor::ALL.into_iter().find(|a| a.name() == s)
    }

    /// (horizontal, vertical)
    pub fn axes(self) -> (Axis, Axis) {
        let i = Anchor::ALL.iter().position(|a| *a == self).expect("in ALL");
        let axis = |n: usize| [Axis::Start, Axis::Center, Axis::End][n];
        (axis(i % 3), axis(i / 3))
    }

    pub fn from_axes(h: Axis, v: Axis) -> Anchor {
        let n = |a: Axis| match a {
            Axis::Start => 0,
            Axis::Center => 1,
            Axis::End => 2,
        };
        Anchor::ALL[n(v) * 3 + n(h)]
    }
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An anchor plus an offset in logical pixels, measured inward from the
/// anchor edge(s); along a centered axis, positive is right/down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub anchor: Anchor,
    pub offset: [i32; 2],
}

impl Default for Position {
    fn default() -> Self {
        Position {
            anchor: Anchor::TopRight,
            offset: [16, 16],
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:+}{:+}",
            self.anchor, self.offset[0], self.offset[1]
        )
    }
}

/// The badge rectangle for `pos`, clamped into `usable`. The saved position
/// itself is never changed by clamping (SPEC §6.1).
pub fn place(
    pos: Position,
    width: u32,
    height: u32,
    usable: PhysicalRect,
    scale: f64,
) -> PhysicalRect {
    let (h, v) = pos.anchor.axes();
    // In i64, so no offset can overflow before the clamp.
    let along = |axis: Axis, start: i32, area: u32, len: u32, off: i32| {
        let (start, area, len) = (i64::from(start), i64::from(area), i64::from(len));
        let off = (f64::from(off) * scale).round() as i64;
        let pos = match axis {
            Axis::Start => start + off,
            Axis::End => start + area - len - off,
            Axis::Center => start + (area - len) / 2 + off,
        };
        pos.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    let x = along(h, usable.x, usable.width, width, pos.offset[0]);
    let y = along(v, usable.y, usable.height, height, pos.offset[1]);
    PhysicalRect::new(x, y, width, height).clamped_into(usable)
}

/// Snap a dropped badge: the anchor is the ninth of `usable` containing the
/// badge's center, and the offset is recomputed from it (SPEC §6.3).
pub fn snap(rect: PhysicalRect, usable: PhysicalRect, scale: f64) -> Position {
    let c = rect.center();
    let third = |pos: i32, start: i32, len: u32| match ((i64::from(pos - start) * 3)
        / i64::from(len.max(1)))
    .clamp(0, 2)
    {
        0 => Axis::Start,
        1 => Axis::Center,
        _ => Axis::End,
    };
    let (h, v) = (
        third(c.x, usable.x, usable.width),
        third(c.y, usable.y, usable.height),
    );
    let off = |axis: Axis, pos: i32, len: u32, start: i32, area: u32| match axis {
        Axis::Start => pos - start,
        Axis::End => start + area as i32 - (pos + len as i32),
        Axis::Center => pos - (start + (area as i32 - len as i32) / 2),
    };
    let to_logical = |p: i32| (f64::from(p) / scale).round() as i32;
    Position {
        anchor: Anchor::from_axes(h, v),
        offset: [
            to_logical(off(h, rect.x, rect.width, usable.x, usable.width)),
            to_logical(off(v, rect.y, rect.height, usable.y, usable.height)),
        ],
    }
}

/// The `label_size` setting: a cap on the label's glyph height.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LabelSize {
    #[default]
    Auto,
    Small,
    Medium,
    Large,
}

impl LabelSize {
    pub fn parse(s: &str) -> Option<LabelSize> {
        match s {
            "auto" => Some(LabelSize::Auto),
            "small" => Some(LabelSize::Small),
            "medium" => Some(LabelSize::Medium),
            "large" => Some(LabelSize::Large),
            _ => None,
        }
    }

    /// The tallest glyph allowed, in logical pixels; `None` means no cap.
    pub fn cap(self) -> Option<u32> {
        match self {
            LabelSize::Auto => None,
            LabelSize::Small => Some(7),
            LabelSize::Medium => Some(14),
            LabelSize::Large => Some(21),
        }
    }
}

impl fmt::Display for LabelSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LabelSize::Auto => "auto",
            LabelSize::Small => "small",
            LabelSize::Medium => "medium",
            LabelSize::Large => "large",
        })
    }
}

/// Physical pixels per art pixel: `max(1, round(size / 32 × display_scale))`.
pub fn art_scale(size: u32, display_scale: f64) -> u32 {
    ((f64::from(size) / f64::from(FRAME_SIZE) * display_scale).round() as u32).max(1)
}

/// The label strip on the crate front, in art pixels: x, y, width, height.
pub const STRIP: (u32, u32, u32, u32) = (2, 22, 28, 9);

/// Clear space the text keeps inside every edge of the strip, in art pixels.
pub const STRIP_MARGIN: u32 = 1;

/// Which bitmap font the label is set in. On a tie in glyph height the
/// regular font wins, so it sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Face {
    /// 5 × 7.
    Regular,
    /// 3 × 5.
    Micro,
}

impl Face {
    pub fn metrics(self) -> Metrics {
        match self {
            Face::Regular => Metrics::REGULAR,
            Face::Micro => Metrics::MICRO,
        }
    }
}

/// How the label is set on the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelFit {
    pub face: Face,
    /// Physical pixels per font pixel.
    pub scale: u32,
    /// How many characters are drawn, counted from the left.
    pub chars: usize,
    /// Top-left of the first glyph, relative to the badge origin.
    pub at: Point,
}

/// Set an `n`-character label on the strip at `p` physical pixels per art
/// pixel (SPEC §4.4). Of the two fonts at every integer scale whose glyphs
/// fit the strip's height and the `label_size` cap, the tallest that fits
/// the whole label wins (the regular font on a tie), centered. If none
/// does, the micro font at scale 1 shows as many whole characters from the
/// left as fit, left-aligned.
pub fn fit_label(n: usize, p: u32, label_size: LabelSize, display_scale: f64) -> LabelFit {
    let (sx, sy, sw, sh) = STRIP;
    let room_w = (sw - 2 * STRIP_MARGIN) * p;
    let room_h = (sh - 2 * STRIP_MARGIN) * p;
    let cap = label_size.cap().map(|c| f64::from(c) * display_scale);
    let height = |face: Face, f: u32| face.metrics().height * f;
    let mut candidates: Vec<(Face, u32)> = [Face::Regular, Face::Micro]
        .into_iter()
        .flat_map(|face| {
            (1..)
                .take_while(move |&f| height(face, f) <= room_h)
                .map(move |f| (face, f))
        })
        .filter(|&(face, f)| cap.is_none_or(|c| f64::from(height(face, f)) <= c))
        .collect();
    candidates.sort_by_key(|&(face, f)| (std::cmp::Reverse(height(face, f)), face));
    let (face, scale, chars) = candidates
        .into_iter()
        .find(|&(face, f)| face.metrics().text_width(n) * f <= room_w)
        .map(|(face, f)| (face, f, n))
        .unwrap_or_else(|| {
            let fits = (0..=n)
                .rev()
                .find(|&k| Metrics::MICRO.text_width(k) <= room_w)
                .unwrap_or(0);
            (Face::Micro, 1, fits)
        });
    let text_w = face.metrics().text_width(chars) * scale;
    let x = if chars == n {
        sx * p + (sw * p - text_w) / 2
    } else {
        (sx + STRIP_MARGIN) * p
    };
    let y = sy * p + (sh * p - height(face, scale)) / 2;
    LabelFit {
        face,
        scale,
        chars,
        at: Point::new(x as i32, y as i32),
    }
}

/// Where everything goes inside the badge image, in physical pixels. The
/// badge is always exactly the 32 × 32 art canvas times `art_scale`, label
/// included (SPEC §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadgeLayout {
    /// The badge's width and height.
    pub side: u32,
    pub art_scale: u32,
    pub label: Option<LabelFit>,
}

/// Lay out the badge (SPEC §4.4): the label, if any, on the strip.
pub fn badge_layout(
    size: u32,
    label_chars: Option<usize>,
    label_size: LabelSize,
    display_scale: f64,
) -> BadgeLayout {
    let p = art_scale(size, display_scale);
    BadgeLayout {
        side: FRAME_SIZE * p,
        art_scale: p,
        label: label_chars
            .filter(|&n| n > 0)
            .map(|n| fit_label(n, p, label_size, display_scale)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USABLE: PhysicalRect = PhysicalRect::new(0, 0, 1920, 1040);

    #[test]
    fn anchor_names_and_axes_round_trip() {
        for a in Anchor::ALL {
            assert_eq!(Anchor::parse(a.name()), Some(a));
            let (h, v) = a.axes();
            assert_eq!(Anchor::from_axes(h, v), a);
        }
        assert_eq!(Anchor::TopRight.axes(), (Axis::End, Axis::Start));
        assert_eq!(Anchor::parse("middle"), None);
    }

    #[test]
    fn placement_per_anchor() {
        let pos = |anchor| Position {
            anchor,
            offset: [16, 16],
        };
        let cases = [
            (Anchor::TopRight, (1840, 16)),
            (Anchor::TopLeft, (16, 16)),
            (Anchor::BottomRight, (1840, 960)),
            (Anchor::BottomLeft, (16, 960)),
            (Anchor::Top, (944, 16)),
            (Anchor::Center, (944, 504)),
            (Anchor::Right, (1840, 504)),
        ];
        for (anchor, (x, y)) in cases {
            assert_eq!(
                place(pos(anchor), 64, 64, USABLE, 1.0),
                PhysicalRect::new(x, y, 64, 64),
                "{anchor}"
            );
        }
        // Offsets are logical: at 2x they double.
        assert_eq!(
            place(
                Position::default(),
                128,
                128,
                PhysicalRect::new(0, 0, 3840, 2160),
                2.0
            ),
            PhysicalRect::new(3680, 32, 128, 128)
        );
    }

    #[test]
    fn placement_clamps_without_changing_position() {
        let far = Position {
            anchor: Anchor::TopLeft,
            offset: [5000, 16],
        };
        assert_eq!(
            place(far, 64, 64, USABLE, 1.0),
            PhysicalRect::new(1856, 16, 64, 64)
        );
    }

    #[test]
    fn extreme_offsets_clamp_without_overflow() {
        for anchor in Anchor::ALL {
            for off in [i32::MAX, i32::MIN] {
                let pos = Position {
                    anchor,
                    offset: [off, off],
                };
                let rect = place(pos, 64, 64, USABLE, 2.0);
                assert_eq!(rect.clamped_into(USABLE), rect, "{pos}");
            }
        }
    }

    #[test]
    fn snap_round_trips_every_anchor() {
        for anchor in Anchor::ALL {
            for scale in [1.0, 1.5, 2.0] {
                let pos = Position {
                    anchor,
                    offset: [16, 24],
                };
                let usable = PhysicalRect::new(100, 30, 2560, 1400);
                let rect = place(pos, 96, 128, usable, scale);
                assert_eq!(snap(rect, usable, scale), pos, "{anchor} at {scale}");
            }
        }
    }

    #[test]
    fn snap_picks_the_ninth_containing_the_center() {
        let cases = [
            (PhysicalRect::new(10, 10, 64, 64), Anchor::TopLeft, [10, 10]),
            (
                PhysicalRect::new(1800, 900, 64, 64),
                Anchor::BottomRight,
                [56, 76],
            ),
            (PhysicalRect::new(900, 10, 64, 64), Anchor::Top, [-28, 10]),
            (
                PhysicalRect::new(1250, 400, 64, 64),
                Anchor::Right,
                [606, -88],
            ),
        ];
        for (rect, anchor, offset) in cases {
            assert_eq!(
                snap(rect, USABLE, 1.0),
                Position { anchor, offset },
                "{rect:?}"
            );
        }
    }

    #[test]
    fn scales() {
        assert_eq!(art_scale(64, 1.0), 2);
        assert_eq!(art_scale(64, 1.25), 3); // round(2.5) = 3
        assert_eq!(art_scale(32, 1.0), 1);
        assert_eq!(art_scale(128, 1.5), 6);
        assert_eq!(art_scale(32, 0.5), 1);
    }

    /// (face, scale, characters drawn, x, y) for `label` at `p`, uncapped.
    fn fit(label: &str, p: u32) -> (Face, u32, usize, i32, i32) {
        let l = fit_label(label.chars().count(), p, LabelSize::Auto, 1.0);
        (l.face, l.scale, l.chars, l.at.x, l.at.y)
    }

    /// Just the face and scale.
    fn fs(label: &str, p: u32) -> (Face, u32) {
        let (face, scale, ..) = fit(label, p);
        (face, scale)
    }

    use Face::{Micro, Regular};

    #[test]
    fn the_tallest_rendering_that_fits_wins() {
        // P = 1: the strip is 28 x 9 physical, 26 x 7 inside the margin.
        // OWL in 5 x 7 is 17 wide; centered, one clear row above and below.
        assert_eq!(fit("OWL", 1), (Regular, 1, 3, 2 + (28 - 17) / 2, 23));
        // MOOSE in 5 x 7 is 29 > 26; in 3 x 5 it is 19.
        assert_eq!(fit("MOOSE", 1), (Micro, 1, 5, 2 + (28 - 19) / 2, 22 + 2));
        // P = 2: room 52 x 14. OWL at 5 x 7 scale 2 is 34 x 14.
        assert_eq!(fit("OWL", 2), (Regular, 2, 3, 4 + (56 - 34) / 2, 44 + 2));
        // MOOSE: 5 x 7 scale 2 is 58 > 52; 3 x 5 scale 2 is 38 x 10.
        assert_eq!(fit("MOOSE", 2), (Micro, 2, 5, 4 + (56 - 38) / 2, 44 + 4));
        // P = 4: room 104 x 28, so OWL fills the strip's height at scale 4.
        assert_eq!(fit("OWL", 4), (Regular, 4, 3, 8 + (112 - 68) / 2, 88 + 4));
        // ARMADILLO at P = 4: 3 x 5 scale 2 (10 tall, 70 wide) beats 5 x 7
        // scale 1 (7 tall, 53 wide).
        assert_eq!(fs("ARMADILLO", 4), (Micro, 2));
    }

    #[test]
    fn hostname_labels_at_every_scale() {
        // PRODOWL02 (from prod-owl-02): 53 wide in 5 x 7, 35 in 3 x 5.
        assert_eq!(
            fit("PRODOWL02", 2),
            (Micro, 1, 9, 4 + (56 - 35) / 2, 44 + 6)
        );
        // P = 3 (64 at 150 %): room 78 x 21; 3 x 5 scale 2 is 70 wide.
        assert_eq!(fs("PRODOWL02", 3), (Micro, 2));
        // P = 4: 3 x 5 scale 3 is 105 > 104, 5 x 7 scale 2 is 106 > 104.
        assert_eq!(fs("PRODOWL02", 4), (Micro, 2));
        // A 20-character label only fits fully in 3 x 5 scale 1 (79 wide).
        let twenty = "ABCDEFGHIJKLMNOPQRST";
        assert_eq!(fit(twenty, 4), (Micro, 1, 20, 8 + (112 - 79) / 2, 88 + 15));
        // At P = 6 (128 at 150 %) the 5 x 7 font fits it (119 <= 156).
        assert_eq!(fs(twenty, 6), (Regular, 1));
        assert_eq!(fit("OWL02", 1), (Micro, 1, 5, 6, 24));
    }

    #[test]
    fn too_long_labels_keep_their_left_side() {
        // P = 1: 6 whole 3 x 5 glyphs (23 px) fit in 26; 7 (27) do not.
        // Left-aligned at the margin.
        assert_eq!(fit("PRODOWL02", 1), (Micro, 1, 6, 3, 24));
        assert_eq!(fit("JENKINSAGENTLINUX07", 1).2, 6);
        // P = 2: room 52, so 13 glyphs (51 px).
        assert_eq!(fit("JENKINSAGENTLINUX07", 2), (Micro, 1, 13, 4 + 2, 44 + 6));
        // P = 3: room 78: 19 glyphs (75) fit, a 20th (79) does not.
        assert_eq!(fit("JENKINSAGENTLINUX07", 3).2, 19);
        assert_eq!(fit("ABCDEFGHIJKLMNOPQRST", 3), (Micro, 1, 19, 9, 66 + 11));
    }

    #[test]
    fn label_size_caps_the_glyph_height() {
        let capped = |size: LabelSize, p: u32, scale: f64| {
            let l = fit_label(3, p, size, scale);
            (l.face, l.scale)
        };
        // OWL at 128 (P = 4): uncapped it is 28 tall.
        assert_eq!(capped(LabelSize::Auto, 4, 1.0), (Regular, 4));
        assert_eq!(capped(LabelSize::Small, 4, 1.0), (Regular, 1));
        assert_eq!(capped(LabelSize::Medium, 4, 1.0), (Regular, 2));
        assert_eq!(capped(LabelSize::Large, 4, 1.0), (Regular, 3));
        // The cap is logical: at 200 %, small allows 14 physical px.
        assert_eq!(capped(LabelSize::Small, 4, 2.0), (Regular, 2));
        // At 125 %, small allows 8.75: still 5 x 7 scale 1.
        assert_eq!(capped(LabelSize::Small, 4, 1.25), (Regular, 1));
        // A cap never makes a label vanish: below 5 px, 3 x 5 scale 1 stays.
        assert_eq!(capped(LabelSize::Small, 1, 0.5), (Micro, 1));
        // A cap never overrides fitting: large at P = 1 is still 7 tall.
        assert_eq!(capped(LabelSize::Large, 1, 1.0), (Regular, 1));
    }

    #[test]
    fn the_badge_is_always_the_art_canvas_times_p() {
        for scale in [0.5, 1.0, 1.25, 1.5, 2.0] {
            for size in [32, 64, 96, 128, 256] {
                let p = art_scale(size, scale);
                for n in [None, Some(0), Some(1), Some(3), Some(9), Some(20), Some(63)] {
                    for ls in [
                        LabelSize::Auto,
                        LabelSize::Small,
                        LabelSize::Medium,
                        LabelSize::Large,
                    ] {
                        let l = badge_layout(size, n, ls, scale);
                        assert_eq!((l.side, l.art_scale), (32 * p, p));
                        // The text stays inside the strip, clear of its margin.
                        if let Some(f) = l.label {
                            let m = f.face.metrics();
                            let (x, y) = (f.at.x as u32, f.at.y as u32);
                            let w = m.text_width(f.chars) * f.scale;
                            let h = m.height * f.scale;
                            let (sx, sy, sw, sh) = STRIP;
                            let inner = |a: u32| (a + STRIP_MARGIN) * p;
                            assert!(x >= inner(sx) && y >= inner(sy), "{size} {n:?} {ls}");
                            assert!(x + w <= (sx + sw - STRIP_MARGIN) * p, "{size} {n:?}");
                            assert!(y + h <= (sy + sh - STRIP_MARGIN) * p, "{size} {n:?}");
                            assert!(f.chars >= 1);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn no_label() {
        assert_eq!(badge_layout(128, None, LabelSize::Auto, 1.0).label, None);
        assert_eq!(badge_layout(64, Some(0), LabelSize::Auto, 1.0).label, None);
    }
}
