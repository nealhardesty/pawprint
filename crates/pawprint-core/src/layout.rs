//! Placement and badge geometry (SPEC §4.2–4.4, §6.1, §6.3): anchor/offset
//! ↔ rectangle, clamping, the 3 × 3 snap, and where the label goes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geom::{PhysicalRect, Point};
use pawprint_sprite::{FRAME_SIZE, Font};

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

/// The `label_size` setting.
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

/// Physical pixels per font pixel (SPEC §4.4).
pub fn font_scale(label_size: LabelSize, size: u32, display_scale: f64) -> u32 {
    let logical = match label_size {
        LabelSize::Small => 1,
        LabelSize::Medium => 2,
        LabelSize::Large => 3,
        LabelSize::Auto if size >= 64 => 2,
        LabelSize::Auto => 1,
    };
    ((f64::from(logical) * display_scale).round() as u32).max(1)
}

/// The label strip on the crate front, in art pixels: x, y, width, height.
pub const STRIP: (u32, u32, u32, u32) = (2, 22, 28, 9);

/// Nameplate dimensions in font pixels, from the crate art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlateMetrics {
    pub left: u32,
    pub right: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Centered on the crate's label strip.
    Strip,
    /// On a nameplate under the crate.
    Under,
    /// On a nameplate beside the crate, toward the screen interior.
    Beside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelBox {
    pub placement: Placement,
    /// Top-left of the first glyph, relative to the badge origin.
    pub text: Point,
    /// The nameplate, relative to the badge origin, unless on the strip.
    pub plate: Option<PhysicalRect>,
    /// Text columns covered by the nameplate's middle section, in font px.
    pub plate_columns: u32,
}

/// Where everything goes inside the badge image, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadgeLayout {
    pub width: u32,
    pub height: u32,
    pub crate_origin: Point,
    pub art_scale: u32,
    pub font_scale: u32,
    pub label: Option<LabelBox>,
}

/// Lay out the badge (SPEC §4.4): the label goes on the strip if it fits,
/// otherwise on a nameplate under the crate, or beside it at size 32 with
/// `label_size = auto`, on the side facing the screen interior.
pub fn badge_layout(
    size: u32,
    label_chars: Option<usize>,
    label_size: LabelSize,
    anchor: Anchor,
    display_scale: f64,
    plate: PlateMetrics,
) -> BadgeLayout {
    let p = art_scale(size, display_scale);
    let f = font_scale(label_size, size, display_scale);
    let crate_px = FRAME_SIZE * p;
    let mut layout = BadgeLayout {
        width: crate_px,
        height: crate_px,
        crate_origin: Point::default(),
        art_scale: p,
        font_scale: f,
        label: None,
    };
    let Some(n) = label_chars.filter(|&n| n > 0) else {
        return layout;
    };

    let cols = Font::text_width(n);
    let (text_w, text_h) = (cols * f, Font::HEIGHT * f);
    let (sx, sy, sw, sh) = STRIP;
    if text_w <= sw * p && text_h <= sh * p {
        let text = Point::new(
            ((sx * p) + (sw * p - text_w) / 2) as i32,
            ((sy * p) + (sh * p - text_h) / 2) as i32,
        );
        layout.label = Some(LabelBox {
            placement: Placement::Strip,
            text,
            plate: None,
            plate_columns: cols,
        });
        return layout;
    }

    let (plate_w, plate_h) = ((plate.left + cols + plate.right) * f, plate.height * f);
    let beside = size == FRAME_SIZE && label_size == LabelSize::Auto;
    let (placement, px, py) = if beside {
        let toward_left = anchor.axes().0 == Axis::End;
        let px = if toward_left {
            -(plate_w as i32)
        } else {
            crate_px as i32
        };
        (Placement::Beside, px, crate_px as i32 - plate_h as i32)
    } else {
        (
            Placement::Under,
            (crate_px as i32 - plate_w as i32) / 2,
            crate_px as i32,
        )
    };
    // Shift everything so the badge image starts at (0, 0).
    let (min_x, min_y) = (px.min(0), py.min(0));
    let max_x = (px + plate_w as i32).max(crate_px as i32);
    let max_y = (py + plate_h as i32).max(crate_px as i32);
    let plate_rect = PhysicalRect::new(px - min_x, py - min_y, plate_w, plate_h);
    let text = Point::new(
        plate_rect.x + (plate.left * f) as i32,
        plate_rect.y + ((plate.height - Font::HEIGHT) / 2 * f) as i32,
    );
    layout.width = (max_x - min_x) as u32;
    layout.height = (max_y - min_y) as u32;
    layout.crate_origin = Point::new(-min_x, -min_y);
    layout.label = Some(LabelBox {
        placement,
        text,
        plate: Some(plate_rect),
        plate_columns: cols,
    });
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    const USABLE: PhysicalRect = PhysicalRect::new(0, 0, 1920, 1040);
    const PLATE: PlateMetrics = PlateMetrics {
        left: 3,
        right: 3,
        height: 13,
    };

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
        assert_eq!(font_scale(LabelSize::Auto, 32, 1.0), 1);
        assert_eq!(font_scale(LabelSize::Auto, 64, 1.0), 2);
        assert_eq!(font_scale(LabelSize::Large, 32, 2.0), 6);
    }

    #[test]
    fn short_labels_sit_on_the_strip() {
        // 64 px, medium: strip is 56 x 18 physical, "OWL" is 34 x 14, so it
        // clears the rail above and the outline below by 2 px.
        let l = badge_layout(64, Some(3), LabelSize::Auto, Anchor::TopRight, 1.0, PLATE);
        assert_eq!(
            (l.width, l.height, l.art_scale, l.font_scale),
            (64, 64, 2, 2)
        );
        let label = l.label.unwrap();
        assert_eq!(label.placement, Placement::Strip);
        assert_eq!(label.text, Point::new(4 + (56 - 34) / 2, 44 + 2));
        // 32 px, small: 4 characters fit (23 <= 28), 5 do not (29), and the
        // 7-row glyphs sit one row clear of the rail in the 9-row strip.
        let small = badge_layout(32, Some(4), LabelSize::Auto, Anchor::TopRight, 1.0, PLATE)
            .label
            .unwrap();
        assert_eq!(small.placement, Placement::Strip);
        assert_eq!(small.text, Point::new(2 + (28 - 23) / 2, 22 + 1));
        assert_eq!(
            badge_layout(128, Some(9), LabelSize::Auto, Anchor::TopRight, 1.0, PLATE)
                .label
                .unwrap()
                .placement,
            Placement::Strip
        );
    }

    #[test]
    fn long_labels_go_on_a_nameplate_under() {
        // 64 px, "PROD-OWL-02": 11 chars → 65 font px → 130 physical, plate (3+65+3)*2 = 142 wide.
        let l = badge_layout(64, Some(11), LabelSize::Auto, Anchor::TopRight, 1.0, PLATE);
        let label = l.label.unwrap();
        assert_eq!(label.placement, Placement::Under);
        assert_eq!(label.plate, Some(PhysicalRect::new(0, 64, 142, 26)));
        assert_eq!((l.width, l.height), (142, 90));
        assert_eq!(
            l.crate_origin,
            Point::new(39, 0),
            "crate centered over the wider plate"
        );
        assert_eq!(label.text, Point::new(6, 64 + 6));
    }

    #[test]
    fn nameplate_goes_beside_at_size_32_toward_the_interior() {
        // 5 chars → 29 font px → plate 35 x 13.
        let right = badge_layout(32, Some(5), LabelSize::Auto, Anchor::TopRight, 1.0, PLATE);
        let label = right.label.unwrap();
        assert_eq!(label.placement, Placement::Beside);
        assert_eq!(
            label.plate,
            Some(PhysicalRect::new(0, 19, 35, 13)),
            "left of the crate for right anchors"
        );
        assert_eq!(right.crate_origin, Point::new(35, 0));
        assert_eq!((right.width, right.height), (67, 32));

        let left = badge_layout(32, Some(5), LabelSize::Auto, Anchor::BottomLeft, 1.0, PLATE);
        assert_eq!(
            left.label.unwrap().plate,
            Some(PhysicalRect::new(32, 19, 35, 13))
        );
        assert_eq!(left.crate_origin, Point::new(0, 0));

        // An explicit label size keeps the plate under.
        let small = badge_layout(32, Some(5), LabelSize::Small, Anchor::TopRight, 1.0, PLATE);
        assert_eq!(small.label.unwrap().placement, Placement::Under);
    }

    #[test]
    fn no_label() {
        let l = badge_layout(128, None, LabelSize::Auto, Anchor::TopRight, 1.0, PLATE);
        assert_eq!((l.width, l.height, l.label), (128, 128, None));
        assert_eq!(
            badge_layout(64, Some(0), LabelSize::Auto, Anchor::TopRight, 1.0, PLATE).label,
            None
        );
    }
}
