//! macOS backend (SPEC §11.2): an accessory app with one borderless,
//! non-activating `NSPanel` at status level, on every Space, showing the
//! frame as a `CGImage` on a nearest-neighbor layer.
//!
//! The loop pulls events with `nextEventMatchingMask` until the core's next
//! deadline. IPC requests and screen changes wake it by posting an
//! application-defined event from the main queue. Every other event goes
//! back to `NSApplication::sendEvent`, so a status item (M4) works as is.

use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSEventModifierFlags, NSEventType, NSPanel, NSResponder, NSScreen,
    NSStatusWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFData, CFRetained, CFType};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSNotification, NSNotificationCenter, NSObject, NSPoint,
    NSProcessInfo, NSRect, NSSize,
};
use objc2_quartz_core::{CALayer, CATransaction, kCAFilterNearest};
use pawprint_core::badge::Event;
use pawprint_core::config::Backend;
use pawprint_core::geom::{PhysicalRect, Point};
use pawprint_core::movemode::{Key, KeyCode};
use pawprint_core::render::Frame;
use pawprint_core::surface::{DisplayInfo, Surface, select_display};

use crate::shell::Shell;
use crate::{CmdResult, doctor::Report, exit, instance, ipc};

pub fn run(mut shell: Shell, listener: ipc::Listener) -> CmdResult {
    let mtm = MainThreadMarker::new().ok_or_else(|| {
        (
            exit::ERROR,
            "the badge must run on the main thread".to_string(),
        )
    })?;
    let configured = shell.configured_backend();
    if !matches!(configured, Backend::Auto | Backend::Macos) {
        log::warn!("backend = \"{configured}\" does not exist on macOS; using macos");
    }
    shell.set_backend("macos");

    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon, no app menu, never the active app (SPEC §11.2).
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    let mut mac = Mac::new(mtm);
    shell.start(&mut mac);

    let (tx, rx) = mpsc::channel();
    ipc::serve(listener, move |incoming| {
        let sent = tx.send(incoming).is_ok();
        if sent {
            wake();
        }
        sent
    });
    let screens_changed = Arc::new(AtomicBool::new(false));
    let flag = screens_changed.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        flag.store(true, Ordering::Relaxed);
        wake();
    });
    // SAFETY: the block only touches an atomic and posts to the main queue,
    // so it is fine on any thread; the observer lives until `run` returns.
    let _observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
            None,
            &block,
        )
    };

    loop {
        while let Ok(incoming) = rx.try_recv() {
            shell.request(&mut mac, incoming);
        }
        if screens_changed.swap(false, Ordering::Relaxed) {
            mac.read_screens();
            let displays = mac.displays.clone();
            shell.handle(&mut mac, Event::DisplaysChanged(displays));
        }
        if shell.done() {
            return Ok(());
        }
        let until = match shell.timeout() {
            Some(t) => NSDate::dateWithTimeIntervalSinceNow(t.as_secs_f64()),
            None => NSDate::distantFuture(),
        };
        // SAFETY: the default run loop mode is a valid mode.
        let mode = unsafe { NSDefaultRunLoopMode };
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&until),
            mode,
            true,
        ) {
            match mac.translate(&event) {
                Translated::Event(e) => shell.handle(&mut mac, e),
                Translated::Swallow => {}
                Translated::Pass => app.sendEvent(&event),
            }
        }
        shell.tick(&mut mac);
    }
}

/// Wake the loop from any thread: the main queue posts an
/// application-defined event, which ends `nextEventMatchingMask`.
fn wake() {
    DispatchQueue::main().exec_async(|| {
        let mtm = MainThreadMarker::new().expect("the main queue runs on the main thread");
        let event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            NSPoint::ZERO,
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            0,
            0,
        );
        if let Some(event) = event {
            NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
        }
    });
}

define_class!(
    // SAFETY: NSPanel has no subclassing requirements, and this adds no
    // ivars and no Drop.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PawprintBadgePanel"]
    struct BadgePanel;

    impl BadgePanel {
        /// Borderless windows refuse key status unless they say otherwise;
        /// move mode makes the panel key for the arrow keys.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }
    }
);

/// Cocoa's desktop is in points with a bottom-left origin on the primary
/// screen; the core's is in physical pixels with a top-left origin. This is
/// the one place the two meet.
///
/// Within a screen, one point is `scale` pixels, so rects on a screen
/// convert both ways exactly. Screen origins are spread by the largest
/// scale, so with mixed scales the screens keep their arrangement and never
/// overlap in physical space (they may leave gaps, which nothing crosses).
#[derive(Debug, Clone, Default, PartialEq)]
struct Desktop {
    /// Height of `screens[0]`, whose bottom-left corner is the origin.
    primary_height: f64,
    /// Each screen's frame and backing scale factor.
    screens: Vec<(NSRect, f64)>,
}

impl Desktop {
    /// Screen `i`'s top-left corner in top-down points, and its scale.
    fn screen(&self, i: usize) -> ((f64, f64), f64) {
        let (f, scale) = self.screens.get(i).copied().unwrap_or((NSRect::ZERO, 1.0));
        let top = self.primary_height - f.origin.y - f.size.height;
        ((f.origin.x, top), scale)
    }

    fn spread(&self) -> f64 {
        self.screens.iter().map(|s| s.1).fold(1.0, f64::max)
    }

    /// A top-down point on screen `i`, in physical pixels.
    fn physical(&self, i: usize, x: f64, y: f64) -> (f64, f64) {
        let ((left, top), scale) = self.screen(i);
        let g = self.spread();
        (g * left + scale * (x - left), g * top + scale * (y - top))
    }

    /// A rect on screen `i` (its frame or visible frame, the badge).
    fn rect_to_physical(&self, i: usize, r: NSRect) -> PhysicalRect {
        let scale = self.screen(i).1;
        let top = self.primary_height - r.origin.y - r.size.height;
        let (x, y) = self.physical(i, r.origin.x, top);
        PhysicalRect::new(
            x.round() as i32,
            y.round() as i32,
            (r.size.width * scale).round() as u32,
            (r.size.height * scale).round() as u32,
        )
    }

    fn point_to_physical(&self, p: NSPoint) -> Point {
        let on = |f: &NSRect| {
            (f.origin.x..=f.origin.x + f.size.width).contains(&p.x)
                && (f.origin.y..=f.origin.y + f.size.height).contains(&p.y)
        };
        let i = self.screens.iter().position(|(f, _)| on(f)).unwrap_or(0);
        let (x, y) = self.physical(i, p.x, self.primary_height - p.y);
        Point::new(x.round() as i32, y.round() as i32)
    }

    fn rect_to_cocoa(&self, r: PhysicalRect) -> NSRect {
        let i = (0..self.screens.len())
            .find(|&i| {
                self.rect_to_physical(i, self.screens[i].0)
                    .contains(r.center())
            })
            .unwrap_or(0);
        let ((left, top), scale) = self.screen(i);
        let g = self.spread();
        let (w, h) = (f64::from(r.width) / scale, f64::from(r.height) / scale);
        let x = left + (f64::from(r.x) - g * left) / scale;
        let y = top + (f64::from(r.y) - g * top) / scale;
        NSRect::new(
            NSPoint::new(x, self.primary_height - y - h),
            NSSize::new(w, h),
        )
    }
}

/// Every screen, primary (`screens[0]`, the one with the menu bar) first.
fn screens(mtm: MainThreadMarker) -> (Desktop, Vec<DisplayInfo>) {
    let screens = NSScreen::screens(mtm);
    let desktop = Desktop {
        primary_height: screens.firstObject().map_or(0.0, |s| s.frame().size.height),
        screens: screens
            .iter()
            .map(|s| (s.frame(), s.backingScaleFactor()))
            .collect(),
    };
    let displays = screens
        .iter()
        .enumerate()
        .map(|(i, s)| DisplayInfo {
            name: s.localizedName().to_string(),
            bounds: desktop.rect_to_physical(i, s.frame()),
            work_area: desktop.rect_to_physical(i, s.visibleFrame()),
            scale: s.backingScaleFactor(),
            primary: i == 0,
        })
        .collect();
    (desktop, displays)
}

/// What the loop does with an event.
enum Translated {
    Event(Event),
    /// Move-mode input the core doesn't use: keep it from AppKit (no beep).
    Swallow,
    Pass,
}

struct Mac {
    mtm: MainThreadMarker,
    panel: Retained<NSPanel>,
    layer: Retained<CALayer>,
    desktop: Desktop,
    displays: Vec<DisplayInfo>,
    shown: bool,
    interactive: bool,
    dragging: bool,
}

impl Mac {
    fn new(mtm: MainThreadMarker) -> Mac {
        let rect = NSRect::new(NSPoint::ZERO, NSSize::new(1.0, 1.0));
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        // SAFETY: NSPanel's designated initializer, with matching types.
        let panel: Retained<BadgePanel> = unsafe {
            msg_send![BadgePanel::alloc(mtm), initWithContentRect: rect, styleMask: style,
                backing: NSBackingStoreType::Buffered, defer: false]
        };
        let panel = Retained::into_super(panel);
        // SAFETY: we keep our own reference, so closing must not release it.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setIgnoresMouseEvents(true);
        panel.setLevel(NSStatusWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setHidesOnDeactivate(false);
        // sharingType stays at its default, readOnly, so screen sharing and
        // remote tools see the badge (F12).

        // A layer-hosting view: the layer is ours, not AppKit's.
        let layer = CALayer::new();
        // SAFETY: kCAFilterNearest is a valid filter name.
        layer.setMagnificationFilter(unsafe { kCAFilterNearest });
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect);
        view.setLayer(Some(&layer));
        view.setWantsLayer(true);
        panel.setContentView(Some(&view));

        let mut mac = Mac {
            mtm,
            panel,
            layer,
            desktop: Desktop::default(),
            displays: Vec::new(),
            shown: false,
            interactive: false,
            dragging: false,
        };
        mac.read_screens();
        mac
    }

    fn read_screens(&mut self) {
        (self.desktop, self.displays) = screens(self.mtm);
    }

    fn translate(&mut self, event: &NSEvent) -> Translated {
        let kind = event.r#type();
        if kind == NSEventType::ApplicationDefined {
            return Translated::Swallow;
        }
        if !self.interactive {
            return Translated::Pass;
        }
        let at = || self.desktop.point_to_physical(NSEvent::mouseLocation());
        let ours = event.windowNumber() == self.panel.windowNumber();
        match kind {
            NSEventType::LeftMouseDown if ours => {
                self.dragging = true;
                Translated::Event(Event::DragStart(at()))
            }
            NSEventType::LeftMouseDragged if self.dragging => {
                Translated::Event(Event::DragMove(at()))
            }
            NSEventType::LeftMouseUp if self.dragging => {
                self.dragging = false;
                Translated::Event(Event::DragEnd(at()))
            }
            NSEventType::KeyDown => match key_code(event.keyCode()) {
                Some(code) => Translated::Event(Event::Key(Key {
                    code,
                    shift: event.modifierFlags().contains(NSEventModifierFlags::Shift),
                })),
                None => Translated::Swallow,
            },
            _ => Translated::Pass,
        }
    }

    /// The color space of the panel's screen, so pixels reach the screen
    /// unconverted, as on Windows and Linux.
    fn color_space(&self) -> Option<CFRetained<CGColorSpace>> {
        let screen = self.panel.screen()?;
        let space = screen.colorSpace()?.CGColorSpace()?;
        Some(space.into())
    }
}

/// Virtual key codes (Carbon `kVK_*`).
fn key_code(code: u16) -> Option<KeyCode> {
    Some(match code {
        123 => KeyCode::Left,
        124 => KeyCode::Right,
        125 => KeyCode::Down,
        126 => KeyCode::Up,
        36 | 76 => KeyCode::Enter,
        53 => KeyCode::Escape,
        _ => return None,
    })
}

/// The frame as a `CGImage`: premultiplied BGRA is premultiplied-first
/// alpha in 32-bit little-endian words (F30).
fn cg_image(frame: &Frame, space: &CGColorSpace) -> Option<CFRetained<CGImage>> {
    let data = CFData::from_bytes(&frame.bgra);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let info = CGBitmapInfo::from_bits_retain(
        CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0,
    );
    let (w, h) = (frame.width as usize, frame.height as usize);
    // SAFETY: the provider holds w * h * 4 bytes; a null decode array is allowed.
    unsafe {
        CGImage::new(
            w,
            h,
            8,
            32,
            w * 4,
            Some(space),
            info,
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

impl Surface for Mac {
    fn displays(&self) -> Vec<DisplayInfo> {
        self.displays.clone()
    }

    fn present(&mut self, frame: &Frame) {
        let srgb = || CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        let Some(image) = self
            .color_space()
            .or_else(srgb)
            .and_then(|space| cg_image(frame, &space))
        else {
            log::error!(
                "macos: cannot create a {}x{} image",
                frame.width,
                frame.height
            );
            return;
        };
        let image: &CFType = &image;
        // No implicit cross-fade: a new frame replaces the old one at once.
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // SAFETY: a CGImage is valid layer contents.
        unsafe { self.layer.setContents(Some(image.as_ref())) };
        CATransaction::commit();
    }

    fn place(&mut self, rect: PhysicalRect) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.panel
            .setFrame_display(self.desktop.rect_to_cocoa(rect), true);
        CATransaction::commit();
        if !self.shown {
            self.panel.orderFrontRegardless();
            self.shown = true;
        }
    }

    fn set_interactive(&mut self, on: bool) {
        self.interactive = on;
        self.dragging = false;
        self.panel.setIgnoresMouseEvents(!on);
        if on {
            // A non-activating panel takes the keyboard without activating
            // the app, so the frontmost app stays frontmost.
            self.panel.makeKeyWindow();
        } else if self.panel.isKeyWindow() {
            // Give the keyboard back: ordering the key window out lets the
            // active app's window take it again.
            self.panel.orderOut(None);
            self.panel.orderFrontRegardless();
        }
    }
}

pub fn doctor(report: &mut Report) {
    report.section("Session");
    report.row(
        "macOS",
        NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string(),
    );
    let session = instance::security_session();
    match session {
        Some(s) => report.row(
            "security session",
            format!(
                "{}{}{}",
                s.id,
                if s.graphic {
                    ", GUI"
                } else {
                    ", no window server"
                },
                if s.remote { ", remote" } else { "" }
            ),
        ),
        None => report.problem("cannot read the security session (SessionGetInfo failed)"),
    }

    report.section("Backend");
    let gui = session.is_some_and(|s| s.graphic);
    if !gui {
        report.problem(
            "this login has no window server (an SSH session?): start PAWPRINT from a \
             Terminal on the desktop",
        );
    }
    report.row("backend", "macos (non-activating NSPanel at status level)");
    let displays = match MainThreadMarker::new() {
        Some(mtm) if gui => screens(mtm).1,
        _ => Vec::new(),
    };
    report.displays(&displays, select_display(&displays, "primary"));
    report.row("menu bar item", "arrives in M4");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    /// A 1440x900-point Retina primary and a 1920x1080 non-Retina screen to
    /// its right, bottoms aligned.
    fn desktop() -> Desktop {
        Desktop {
            primary_height: 900.0,
            screens: vec![
                (rect(0.0, 0.0, 1440.0, 900.0), 2.0),
                (rect(1440.0, 0.0, 1920.0, 1080.0), 1.0),
            ],
        }
    }

    #[test]
    fn cocoa_and_physical_coordinates_meet_in_one_place() {
        let d = desktop();
        // The primary's visible frame below a 25-point menu bar, above a
        // 70-point Dock: physical, top-left, at scale 2.
        assert_eq!(
            d.rect_to_physical(0, rect(0.0, 70.0, 1440.0, 805.0)),
            PhysicalRect::new(0, 50, 2880, 1610)
        );
        // The second screen keeps its place to the right, at its own scale.
        assert_eq!(
            d.rect_to_physical(1, rect(1440.0, 0.0, 1920.0, 1080.0)),
            PhysicalRect::new(2880, -360, 1920, 1080)
        );
        assert_eq!(
            d.rect_to_cocoa(PhysicalRect::new(2700, 66, 160, 192)),
            rect(1350.0, 771.0, 80.0, 96.0)
        );
        assert_eq!(
            d.rect_to_cocoa(PhysicalRect::new(3000, 100, 80, 96)),
            rect(1560.0, 524.0, 80.0, 96.0)
        );
        for (i, r) in [
            (0, PhysicalRect::new(2700, 66, 160, 192)),
            (1, PhysicalRect::new(3000, 100, 80, 96)),
        ] {
            assert_eq!(d.rect_to_physical(i, d.rect_to_cocoa(r)), r);
        }
        // The pointer: the primary's top-left corner, and a point on the
        // second screen.
        assert_eq!(
            d.point_to_physical(NSPoint::new(0.0, 900.0)),
            Point::new(0, 0)
        );
        assert_eq!(
            d.point_to_physical(NSPoint::new(1500.0, 1000.0)),
            Point::new(2940, -280)
        );
    }

    #[test]
    fn move_mode_keys() {
        assert_eq!(key_code(123), Some(KeyCode::Left));
        assert_eq!(key_code(126), Some(KeyCode::Up));
        assert_eq!(key_code(76), Some(KeyCode::Enter));
        assert_eq!(key_code(53), Some(KeyCode::Escape));
        assert_eq!(key_code(0), None);
    }
}
