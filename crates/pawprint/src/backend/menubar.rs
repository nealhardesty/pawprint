//! The macOS menu-bar item (SPEC §11.2): an `NSStatusItem` showing the
//! mascot's rest frame, with `pawprint_core::menu`'s model as its `NSMenu`.
//! Each choice goes into the badge loop's queue as an [`Inbox::Menu`]
//! command, the same queue IPC uses, and the loop hands it to
//! `Shell::menu`, never from inside the AppKit callback.

use std::cell::RefCell;
use std::sync::mpsc::Sender;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSBitmapImageRep, NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem,
    NSSquareStatusItemLength, NSStatusBar, NSStatusItem,
};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
use objc2_foundation::{NSObject, NSSize, NSString, ns_string};
use pawprint_core::badge::Command;
use pawprint_core::menu::{MenuItem, TrayState};
use pawprint_core::render::Frame;
use pawprint_core::sprite::RgbaImage;

use super::macos::{Inbox, cg_image, wake};
use crate::shell::Tray;

/// The icon's box in points: the menu bar's standard image height.
const ICON_POINTS: u32 = 18;

pub struct MenuBar {
    mtm: MainThreadMarker,
    item: Option<Retained<NSStatusItem>>,
    target: Retained<Target>,
    /// The icon last drawn, so a menu-only change keeps the image.
    icon: Option<RgbaImage>,
}

impl MenuBar {
    pub fn new(mtm: MainThreadMarker, tx: Sender<Inbox>) -> MenuBar {
        MenuBar {
            mtm,
            item: None,
            target: Target::new(mtm, tx),
            icon: None,
        }
    }
}

impl Tray for MenuBar {
    fn show(&mut self, state: &TrayState) {
        if !state.visible {
            if let Some(item) = self.item.take() {
                log::info!("menu bar item: off (tray = false)");
                NSStatusBar::systemStatusBar().removeStatusItem(&item);
                self.icon = None;
            }
            return;
        }
        let item = self.item.get_or_insert_with(|| {
            log::info!("menu bar item: on");
            NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength)
        });
        if let Some(button) = item.button(self.mtm) {
            if self.icon.as_ref() != Some(&state.icon) {
                button.setImage(Some(&icon(&state.icon)));
                self.icon = Some(state.icon.clone());
            }
            button.setToolTip(Some(&NSString::from_str(&state.tooltip)));
        }
        let mut commands = Vec::new();
        let menu = build_menu(self.mtm, &state.menu, &self.target, &mut commands);
        *self.target.ivars().commands.borrow_mut() = commands;
        item.setMenu(Some(&menu));
    }
}

/// `items` as an `NSMenu`. Each item's tag indexes `commands`, which the
/// target reads when the item is chosen.
fn build_menu(
    mtm: MainThreadMarker,
    items: &[MenuItem],
    target: &Target,
    commands: &mut Vec<Command>,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for m in items {
        // SAFETY: `choose:` is the target's action method.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&m.label),
                None,
                ns_string!(""),
            )
        };
        if !m.children.is_empty() {
            item.setSubmenu(Some(&build_menu(mtm, &m.children, target, commands)));
        } else if let Some(command) = m.command {
            item.setTag(commands.len() as isize);
            commands.push(command);
            // SAFETY: the target implements `choose:` and outlives the menu
            // (the menu bar item keeps it).
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(choose:)));
            }
        }
        if let Some(on) = m.checked {
            item.setState(if on {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        item.setEnabled(true);
        menu.addItem(&item);
    }
    menu
}

/// The rest frame drawn into an [`ICON_POINTS`] box with a one-point
/// margin, at `scale` pixels per point, nearest neighbor (32 art pixels
/// fill 16 points: exact at 2x, every other pixel at 1x). Colorful, so not
/// a template image.
fn icon_pixels(icon: &RgbaImage, scale: u32) -> RgbaImage {
    let size = ICON_POINTS * scale;
    let mut out = RgbaImage::new(size, size);
    for y in 0..size - 2 * scale {
        for x in 0..size - 2 * scale {
            let p = icon.pixel(x * 2 / scale, y * 2 / scale);
            if p[3] != 0 {
                out.put(i64::from(x + scale), i64::from(y + scale), p);
            }
        }
    }
    out
}

/// The icon as an `NSImage` with a 1x and a 2x bitmap, so AppKit picks the
/// pixels instead of interpolating.
fn icon(img: &RgbaImage) -> Retained<NSImage> {
    let points = NSSize::new(f64::from(ICON_POINTS), f64::from(ICON_POINTS));
    let image = NSImage::initWithSize(NSImage::alloc(), points);
    // SAFETY: kCGColorSpaceSRGB is a valid color space name.
    let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
    for scale in [1, 2] {
        let frame = Frame::from_rgba(&icon_pixels(img, scale));
        if let Some(cg) = srgb.as_deref().and_then(|s| cg_image(&frame, s)) {
            let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
            rep.setSize(points);
            image.addRepresentation(&rep);
        }
    }
    image
}

pub struct TargetIvars {
    tx: Sender<Inbox>,
    commands: RefCell<Vec<Command>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PawprintMenuTarget"]
    #[ivars = TargetIvars]
    pub struct Target;

    impl Target {
        /// A menu item was chosen: queue its command and wake the loop.
        #[unsafe(method(choose:))]
        fn choose(&self, sender: Option<&AnyObject>) {
            let Some(item) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) else {
                return;
            };
            let command = self.ivars().commands.borrow().get(item.tag() as usize).copied();
            if let Some(command) = command
                && self.ivars().tx.send(Inbox::Menu(command)).is_ok()
            {
                wake();
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, tx: Sender<Inbox>) -> Retained<Target> {
        let this = Target::alloc(mtm).set_ivars(TargetIvars {
            tx,
            commands: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_nearest_neighbor_with_a_margin() {
        let mut art = RgbaImage::new(32, 32);
        art.put(0, 0, [255, 0, 0, 255]);
        art.put(1, 0, [0, 255, 0, 255]);
        art.put(31, 31, [0, 0, 255, 255]);
        let x2 = icon_pixels(&art, 2);
        assert_eq!((x2.width, x2.height), (36, 36));
        assert_eq!(x2.pixel(1, 1), [0, 0, 0, 0], "margin");
        assert_eq!(x2.pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(x2.pixel(3, 2), [0, 255, 0, 255]);
        assert_eq!(x2.pixel(33, 33), [0, 0, 255, 255]);
        let x1 = icon_pixels(&art, 1);
        assert_eq!((x1.width, x1.height), (18, 18));
        assert_eq!(x1.pixel(1, 1), [255, 0, 0, 255], "every other art pixel");
        assert_eq!(x1.pixel(2, 1), [0, 0, 0, 0]);
    }
}
