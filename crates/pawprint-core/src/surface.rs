//! The narrow interface every backend implements (SPEC §10.6), the shared
//! effect executor, and a `FakeSurface` for testing without a window.

use crate::badge::Effect;
use crate::config::Fullscreen;
use crate::geom::PhysicalRect;
use crate::render::Frame;

/// One display as the backend reports it, in physical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayInfo {
    /// The name `pawprint doctor` shows and `display = "..."` matches.
    pub name: String,
    pub bounds: PhysicalRect,
    /// Bounds minus taskbars, menu bar, Dock, and panels.
    pub work_area: PhysicalRect,
    /// OS scale factor (1.0, 1.25, 2.0, ...).
    pub scale: f64,
    pub primary: bool,
}

pub trait Surface {
    fn displays(&self) -> Vec<DisplayInfo>;
    /// Show a frame: premultiplied BGRA, already scaled.
    fn present(&mut self, frame: &Frame);
    fn place(&mut self, rect: PhysicalRect);
    /// Move mode on (accept the pointer) or off (click-through).
    fn set_interactive(&mut self, on: bool);
    /// `Hide`: never show over a fullscreen window on the badge's display
    /// (SPEC §6.5). Sent before the first `place` and on every change.
    fn set_fullscreen(&mut self, policy: Fullscreen);
}

/// Execute the surface effects and hand back the rest (tray, state,
/// reload, exit) for the shell. Every backend runs effects through this.
pub fn apply(surface: &mut impl Surface, effects: Vec<Effect>) -> Vec<Effect> {
    let mut rest = Vec::new();
    for effect in effects {
        match effect {
            Effect::Present(frame) => surface.present(&frame),
            Effect::Place(rect) => surface.place(rect),
            Effect::SetInteractive(on) => surface.set_interactive(on),
            Effect::SetFullscreen(policy) => surface.set_fullscreen(policy),
            other => rest.push(other),
        }
    }
    rest
}

/// The target display (SPEC §6.2): the named one if it exists, else the
/// primary, else the first in enumeration order (which covers "the only one").
pub fn select_display<'a>(displays: &'a [DisplayInfo], wanted: &str) -> Option<&'a DisplayInfo> {
    let named = (wanted != "primary")
        .then(|| displays.iter().find(|d| d.name == wanted))
        .flatten();
    named
        .or_else(|| displays.iter().find(|d| d.primary))
        .or_else(|| displays.first())
}

/// Whether a fullscreen window at `window` covers the badge at `badge`:
/// the window spans the whole display the badge's center is on (SPEC §6.5).
/// Backends that watch for fullscreen windows themselves (Windows, X11)
/// ask this, so the rule is written once.
pub fn fullscreen_covers(
    window: PhysicalRect,
    badge: PhysicalRect,
    displays: &[DisplayInfo],
) -> bool {
    let covers = |d: PhysicalRect| {
        window.x <= d.x
            && window.y <= d.y
            && window.right() >= d.right()
            && window.bottom() >= d.bottom()
    };
    displays
        .iter()
        .any(|d| d.bounds.contains(badge.center()) && covers(d.bounds))
}

/// A surface that records what it was asked to do.
#[derive(Debug, Default)]
pub struct FakeSurface {
    pub screens: Vec<DisplayInfo>,
    pub frames: Vec<Frame>,
    pub places: Vec<PhysicalRect>,
    pub interactive: bool,
    pub fullscreen: Option<Fullscreen>,
}

impl FakeSurface {
    pub fn new(screens: Vec<DisplayInfo>) -> FakeSurface {
        FakeSurface {
            screens,
            ..FakeSurface::default()
        }
    }

    pub fn rect(&self) -> Option<PhysicalRect> {
        self.places.last().copied()
    }
}

impl Surface for FakeSurface {
    fn displays(&self) -> Vec<DisplayInfo> {
        self.screens.clone()
    }

    fn present(&mut self, frame: &Frame) {
        self.frames.push(frame.clone());
    }

    fn place(&mut self, rect: PhysicalRect) {
        self.places.push(rect);
    }

    fn set_interactive(&mut self, on: bool) {
        self.interactive = on;
    }

    fn set_fullscreen(&mut self, policy: Fullscreen) {
        self.fullscreen = Some(policy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &str, primary: bool) -> DisplayInfo {
        let r = PhysicalRect::new(0, 0, 100, 100);
        DisplayInfo {
            name: name.into(),
            bounds: r,
            work_area: r,
            scale: 1.0,
            primary,
        }
    }

    #[test]
    fn display_selection_order() {
        let ds = [d("HDMI-1", false), d("DP-1", true)];
        assert_eq!(select_display(&ds, "primary").unwrap().name, "DP-1");
        assert_eq!(select_display(&ds, "HDMI-1").unwrap().name, "HDMI-1");
        assert_eq!(
            select_display(&ds, "VGA-9").unwrap().name,
            "DP-1",
            "missing name falls back to primary"
        );
        let none_primary = [d("A", false), d("B", false)];
        assert_eq!(select_display(&none_primary, "primary").unwrap().name, "A");
        assert!(select_display(&[], "primary").is_none());
    }

    #[test]
    fn fullscreen_windows_cover_the_badges_display_only() {
        let left = DisplayInfo {
            bounds: PhysicalRect::new(0, 0, 1920, 1080),
            ..d("A", true)
        };
        let right = DisplayInfo {
            bounds: PhysicalRect::new(1920, 0, 2560, 1440),
            ..d("B", false)
        };
        let ds = [left, right];
        let badge = PhysicalRect::new(1840, 16, 64, 64);
        let on_left = PhysicalRect::new(0, 0, 1920, 1080);
        assert!(fullscreen_covers(on_left, badge, &ds));
        assert!(
            fullscreen_covers(PhysicalRect::new(-8, -8, 1936, 1096), badge, &ds),
            "a window larger than the display still covers it"
        );
        assert!(
            !fullscreen_covers(PhysicalRect::new(1920, 0, 2560, 1440), badge, &ds),
            "fullscreen on the other display"
        );
        assert!(
            !fullscreen_covers(PhysicalRect::new(0, 0, 1920, 1040), badge, &ds),
            "a maximized window leaves the taskbar"
        );
        assert!(
            fullscreen_covers(PhysicalRect::new(0, 0, 4480, 1440), badge, &ds),
            "spanning every display (multi-monitor RDP)"
        );
    }
}
