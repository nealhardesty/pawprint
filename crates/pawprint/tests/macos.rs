//! The macOS backend in a real GUI session (SPEC §11.2). Needs a logged-in
//! desktop, which GitHub's macOS runners have; run with
//! `make test-macos-display`.
//!
//! The window server is asked about the badge's window directly
//! (`CGWindowListCopyWindowInfo`). Checks that need a privacy permission
//! (Screen Recording for the pixel check, Accessibility for synthetic drags
//! and keys) are skipped, with a message, when it isn't granted.

#![cfg(target_os = "macos")]

mod common;

use std::time::{Duration, Instant};

use common::{Env, Image, Proc, wait_for};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, CGPreflightPostEventAccess,
    CGPreflightScreenCaptureAccess, CGWindowListCopyWindowInfo, CGWindowListOption,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, ns_string};

/// `kCGStatusWindowLevel`.
const STATUS_LEVEL: i64 = 25;

/// What the window server says about one window.
#[derive(Debug)]
struct WindowInfo {
    layer: i64,
    /// Global points, top-left origin: x, y, width, height.
    bounds: [f64; 4],
    sharing: i64,
}

fn number(dict: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<f64> {
    let value: Retained<AnyObject> = dict.objectForKey(key)?;
    Some(value.downcast_ref::<NSNumber>()?.as_f64())
}

/// The on-screen window that `pid` owns, if there is exactly one.
fn window_of(pid: u32) -> Option<WindowInfo> {
    let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, 0)?;
    // SAFETY: a CFArray of CFDictionaries is toll-free bridged to NSArray
    // of NSDictionary, and `list` outlives the borrow.
    let list: &NSArray<NSDictionary<NSString, AnyObject>> = unsafe { &*(&raw const *list).cast() };
    let mut ours = list
        .iter()
        .filter(|w| number(w, ns_string!("kCGWindowOwnerPID")) == Some(f64::from(pid)));
    let w = ours.next()?;
    assert!(ours.next().is_none(), "the badge owns one window");
    let bounds = w.objectForKey(ns_string!("kCGWindowBounds"))?;
    let bounds = bounds.downcast_ref::<NSDictionary>()?;
    // SAFETY: the bounds dictionary has NSString keys.
    let bounds: &NSDictionary<NSString, AnyObject> =
        unsafe { &*(bounds as *const NSDictionary).cast() };
    let b = |k: &NSString| number(bounds, k).unwrap_or(f64::NAN);
    Some(WindowInfo {
        layer: number(&w, ns_string!("kCGWindowLayer"))? as i64,
        bounds: [
            b(ns_string!("X")),
            b(ns_string!("Y")),
            b(ns_string!("Width")),
            b(ns_string!("Height")),
        ],
        sharing: number(&w, ns_string!("kCGWindowSharingState"))? as i64,
    })
}

fn status_field(env: &Env, key: &str) -> String {
    let status = env.ok(&["status"]);
    status
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| panic!("no {key:?} in status:\n{status}"))
}

/// The display scale from `pawprint status` ("64x64 at 1200,16 (scale 2)").
fn scale(env: &Env) -> f64 {
    let badge = status_field(env, "badge");
    badge
        .rsplit_once("scale ")
        .and_then(|(_, s)| s.trim_end_matches(')').parse().ok())
        .unwrap_or_else(|| panic!("no scale in {badge:?}"))
}

fn moving(env: &Env) -> bool {
    status_field(env, "move mode") == "on"
}

fn frontmost_pid() -> Option<i32> {
    Some(
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .processIdentifier(),
    )
}

fn post_mouse(kind: CGEventType, at: (f64, f64)) {
    let event = CGEvent::new_mouse_event(None, kind, CGPoint::new(at.0, at.1), CGMouseButton::Left);
    CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
    std::thread::sleep(Duration::from_millis(30));
}

fn post_key(code: u16) {
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(None, code, down);
        CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Screen pixels of the main display, if screen capture is allowed.
fn capture(env: &Env) -> Option<Image> {
    let path = env.dir.join("screen.png");
    let ok = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-x", "-m", "-t", "png"])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    (ok && path.exists()).then(|| Image::load(&path))
}

/// Opaque badge pixels that differ on screen at (x, y).
fn wrong_pixels(screen: &Image, badge: &Image, x: i32, y: i32) -> (usize, usize) {
    let mut wrong = 0;
    let mut total = 0;
    for by in 0..badge.height {
        for bx in 0..badge.width {
            let b = badge.px(bx, by);
            if b[3] != 255 {
                continue;
            }
            total += 1;
            let got = screen.px((x + bx as i32) as u32, (y + by as i32) as u32);
            if got[..3] != b[..3] {
                wrong += 1;
            }
        }
    }
    (wrong, total)
}

fn pixel_check(env: &Env, rect: [i32; 4], scale: f64) {
    if scale != 1.0 {
        eprintln!("SKIP pixel check: the main display is at scale {scale}, the reference at 1");
        return;
    }
    let preflight = CGPreflightScreenCaptureAccess();
    let Some(screen) = capture(env) else {
        eprintln!("SKIP pixel check: screencapture failed (Screen Recording not permitted?)");
        return;
    };
    let expected = env.expected(64);
    let (wrong, total) = wrong_pixels(&screen, &expected, rect[0], rect[1]);
    if wrong == total && !preflight {
        eprintln!(
            "SKIP pixel check: the capture shows no badge and this process has no \
             Screen Recording permission (CGPreflightScreenCaptureAccess is false)"
        );
        return;
    }
    assert_eq!(
        wrong, 0,
        "{wrong} of {total} opaque badge pixels differ on screen at {rect:?}"
    );
    eprintln!("pixel check: all {total} opaque badge pixels match `pawprint render`");
}

/// Move mode with synthetic input: a drag, arrow keys, Enter, and Escape.
fn drag_and_keys(env: &Env, rect: [i32; 4], scale: f64) -> [i32; 4] {
    let pt = |v: i32| f64::from(v) / scale;
    let center = (pt(rect[0] + rect[2] / 2), pt(rect[1] + rect[3] / 2));

    env.ok(&["move"]);
    let to = (center.0 - 300.0, center.1 + 200.0);
    post_mouse(CGEventType::LeftMouseDown, center);
    for i in 1..=10 {
        let f = f64::from(i) / 10.0;
        let at = (
            center.0 + (to.0 - center.0) * f,
            center.1 + (to.1 - center.1) * f,
        );
        post_mouse(CGEventType::LeftMouseDragged, at);
    }
    post_mouse(CGEventType::LeftMouseUp, to);
    let dragged = wait_for("the drag to land", || {
        env.badge_rect().filter(|r| *r != rect && !moving(env))
    });
    let dx = f64::from(dragged[0] - rect[0]) / scale;
    let dy = f64::from(dragged[1] - rect[1]) / scale;
    assert!(
        (dx + 300.0).abs() <= 1.0 && (dy - 200.0).abs() <= 1.0,
        "dragged by ({dx}, {dy}) points, want (-300, 200)"
    );
    assert!(env.state().contains("offset"), "{}", env.state());
    eprintln!("drag: moved to {dragged:?} and saved");

    // An arrow nudges by one logical pixel; Enter saves.
    env.ok(&["move"]);
    post_key(123); // left arrow
    post_key(36); // return
    let s = scale.round() as i32;
    let nudged = [dragged[0] - s, dragged[1], dragged[2], dragged[3]];
    env.wait_rect(nudged);
    wait_for("move mode to end on Enter", || (!moving(env)).then_some(()));

    // Escape cancels a nudge.
    env.ok(&["move"]);
    post_key(124); // right arrow
    env.wait_rect([nudged[0] + s, nudged[1], nudged[2], nudged[3]]);
    post_key(53); // escape
    env.wait_rect(nudged);
    wait_for("move mode to end on Escape", || {
        (!moving(env)).then_some(())
    });
    eprintln!("keys: arrow nudge, Enter, and Escape work");
    nudged
}

#[test]
#[ignore = "needs a logged-in macOS desktop; run with `make test-macos-display`"]
fn badge_in_a_gui_session() {
    let mut env = Env::new("macos");
    env.set("TMPDIR", env.runtime().to_string_lossy());
    let mut badge = env.run_badge();
    let pid = badge.0.id();
    let rect = wait_for("the badge to be placed", || {
        if let Some(code) = badge.exited() {
            panic!("the badge exited ({code}):\n{}", env.log());
        }
        env.badge_rect()
    });
    assert_eq!(status_field(&env, "backend"), "macos");
    assert!(status_field(&env, "session").starts_with("asid-"));
    let scale = scale(&env);
    eprintln!("badge at {rect:?}, scale {scale}");

    // The window: status level, where `status` says, visible to capture.
    let window = wait_for("the badge window", || window_of(pid));
    eprintln!("window: {window:?}");
    assert_eq!(window.layer, STATUS_LEVEL, "status window level");
    assert_ne!(window.sharing, 0, "never sharingType = none (F12)");
    let want = rect.map(|v| f64::from(v) / scale);
    assert!(
        window
            .bounds
            .iter()
            .zip(want)
            .all(|(a, b)| (a - b).abs() < 0.01),
        "window at {:?} points, status says {want:?}",
        window.bounds
    );

    // An accessory app that never becomes frontmost, even in move mode.
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32)
        .expect("the badge is a running application");
    assert_eq!(
        app.activationPolicy(),
        NSApplicationActivationPolicy::Accessory,
        "no Dock icon"
    );
    assert_ne!(
        frontmost_pid(),
        Some(pid as i32),
        "the badge is never frontmost"
    );
    env.ok(&["move"]);
    assert!(moving(&env));
    assert_ne!(
        frontmost_pid(),
        Some(pid as i32),
        "move mode doesn't activate"
    );
    assert!(!app.isActive(), "move mode doesn't activate");
    env.ok(&["move"]); // a second move is harmless

    // One badge per session.
    let second = env.pawprint(&["run"]);
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("already running"));
    assert!(env.ok(&[]).contains("already running"));

    pixel_check(&env, rect, scale);

    // Move mode ends on its own after 30 s; reset it instead of waiting.
    env.ok(&["quit"]);
    wait_for("the badge to quit", || badge.exited());
    let mut badge: Proc = env.run_badge();
    let rect = wait_for("the restarted badge", || {
        if let Some(code) = badge.exited() {
            panic!("the badge exited ({code}):\n{}", env.log());
        }
        env.badge_rect()
    });
    if CGPreflightPostEventAccess() {
        let moved = drag_and_keys(&env, rect, scale);
        // The position survives a restart.
        env.ok(&["quit"]);
        wait_for("the badge to quit", || badge.exited());
        badge = env.run_badge();
        env.wait_rect(moved);
        env.ok(&["reset-position"]);
        env.wait_rect(rect);
    } else {
        eprintln!(
            "SKIP drag and key checks: this process may not post events \
             (CGPreflightPostEventAccess is false; grant Accessibility)"
        );
    }

    // Quit: answered, the process exits cleanly, and the socket is gone.
    env.ok(&["reload"]);
    env.ok(&["quit"]);
    let start = Instant::now();
    let code = loop {
        if let Some(code) = badge.exited() {
            break code;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the badge did not quit"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(code.success(), "exit {code}\n{}", env.log());
    assert_eq!(env.pawprint(&["status"]).status.code(), Some(4));
    let sockets = std::fs::read_dir(env.runtime().join("pawprint"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(sockets, 0, "quit removes the socket");
}

/// The SystemConfiguration hostname (SPEC §5.1) is what `scutil --get`
/// prints: HostName if set, else LocalHostName.
#[test]
#[ignore = "needs a logged-in macOS desktop; run with `make test-macos-display`"]
fn hostname_matches_scutil() {
    let scutil = |key: &str| {
        let out = std::process::Command::new("/usr/sbin/scutil")
            .args(["--get", key])
            .output()
            .unwrap();
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !name.is_empty()).then_some(name)
    };
    let want = scutil("HostName")
        .or_else(|| scutil("LocalHostName"))
        .expect("scutil knows a name");
    let env = Env::new("macos-host");
    let out = env
        .command(env!("CARGO_BIN_EXE_pawprint"))
        .env_remove("PAWPRINT_HOSTNAME")
        .arg("doctor")
        .output()
        .unwrap();
    let doctor = String::from_utf8_lossy(&out.stdout);
    eprintln!("{doctor}");
    let got = doctor
        .lines()
        .find_map(|l| l.trim().strip_prefix("hostname"))
        .map(str::trim);
    assert_eq!(got, Some(want.as_str()));
}
