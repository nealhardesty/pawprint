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
    /// The title; the window server shares it only with Screen Recording
    /// permission.
    name: Option<String>,
    pid: u32,
    #[allow(dead_code)] // shown in failure messages
    owner: String,
}

impl WindowInfo {
    /// A menu-bar item: a thin window along the top of the screen at the
    /// status level, above the menu bar's own window (24).
    fn in_menu_bar(&self) -> bool {
        self.layer >= STATUS_LEVEL && self.at_top()
    }

    /// Along the top of the screen, in the menu bar: menu-bar items, and a
    /// regular app's own menu-bar windows (layer 0).
    fn at_top(&self) -> bool {
        self.bounds[1] < 1.0 && self.bounds[3] <= 40.0
    }
}

fn number(dict: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<f64> {
    let value: Retained<AnyObject> = dict.objectForKey(key)?;
    Some(value.downcast_ref::<NSNumber>()?.as_f64())
}

/// The window server's windows, front to back for on-screen ones.
fn windows(option: CGWindowListOption) -> Vec<WindowInfo> {
    let Some(list) = CGWindowListCopyWindowInfo(option, 0) else {
        return Vec::new();
    };
    // SAFETY: a CFArray of CFDictionaries is toll-free bridged to NSArray
    // of NSDictionary, and `list` outlives the borrow.
    let list: &NSArray<NSDictionary<NSString, AnyObject>> = unsafe { &*(&raw const *list).cast() };
    let text = |w: &NSDictionary<NSString, AnyObject>, k: &NSString| {
        w.objectForKey(k)
            .and_then(|n| n.downcast_ref::<NSString>().map(|s| s.to_string()))
    };
    list.iter()
        .filter_map(|w| {
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
                name: text(&w, ns_string!("kCGWindowName")),
                pid: number(&w, ns_string!("kCGWindowOwnerPID"))? as u32,
                owner: text(&w, ns_string!("kCGWindowOwnerName")).unwrap_or_default(),
            })
        })
        .collect()
}

/// Every on-screen window that `pid` owns.
fn windows_of(pid: u32) -> Vec<WindowInfo> {
    windows(CGWindowListOption::OptionOnScreenOnly)
        .into_iter()
        .filter(|w| w.pid == pid)
        .collect()
}

/// The frontmost window at the normal level belongs to `pid`.
fn in_front(pid: u32) -> bool {
    windows(CGWindowListOption::OptionOnScreenOnly)
        .iter()
        .find(|w| w.layer == 0)
        .is_some_and(|w| w.pid == pid)
}

/// The badge's window: the one window `pid` owns outside the menu bar.
fn window_of(pid: u32) -> Option<WindowInfo> {
    let mut ours: Vec<WindowInfo> = windows_of(pid)
        .into_iter()
        .filter(|w| !w.at_top())
        .collect();
    assert!(ours.len() <= 1, "the badge owns one window: {ours:?}");
    ours.pop()
}

/// Every menu-bar item's window on screen, whoever owns it.
fn menu_bar_items() -> Vec<WindowInfo> {
    windows(CGWindowListOption::OptionOnScreenOnly)
        .into_iter()
        .filter(WindowInfo::in_menu_bar)
        .collect()
}

/// Wait up to 10 s for exactly `n` menu-bar items.
fn menu_bar_count(n: usize, why: &str) -> Vec<WindowInfo> {
    let start = Instant::now();
    loop {
        let items = menu_bar_items();
        if items.len() == n {
            return items;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{why}: want {n} menu-bar items, have {items:#?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The badge's menu-bar item, found by turning it off and on: the window
/// server may give its window to the app or, as on macOS 26, to Control
/// Center, so ownership doesn't tell. It is the rightmost window whose
/// place `tray = false` changed (items to its left shift; items to its
/// right stay). Checks on the way that `tray = false` removes the item and
/// that it comes back.
fn find_menu_bar_item(env: &Env) -> WindowInfo {
    std::thread::sleep(Duration::from_secs(1));
    let on = menu_bar_items();
    assert!(!on.is_empty(), "no menu-bar items at all");
    env.ok(&["set", "tray", "false"]);
    let off = menu_bar_count(on.len() - 1, "tray = false removes the item");
    env.ok(&["unset", "tray"]);
    let back = menu_bar_count(on.len(), "the item comes back");
    let place = |w: &WindowInfo| (w.bounds[0], w.bounds[2]);
    back.into_iter()
        .filter(|w| !off.iter().any(|o| place(o) == place(w)))
        .max_by(|a, b| a.bounds[0].total_cmp(&b.bounds[0]))
        .expect("a menu-bar window that tray = false removed")
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

/// Opaque badge pixels that differ on screen at (x, y). The outer art pixel
/// (2 px at size 64) is skipped: the check runs in move mode, which draws
/// its dashed outline there (SPEC §6.3).
fn wrong_pixels(screen: &Image, badge: &Image, x: i32, y: i32) -> (usize, usize) {
    let mut wrong = 0;
    let mut total = 0;
    for by in 2..badge.height - 2 {
        for bx in 2..badge.width - 2 {
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
    // Like a hand on the mouse: hover the badge first, then press.
    post_mouse(CGEventType::MouseMoved, center);
    std::thread::sleep(Duration::from_millis(200));
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
    let start = Instant::now();
    let dragged = loop {
        if let Some(r) = env.badge_rect().filter(|r| *r != rect && !moving(env)) {
            break r;
        }
        if start.elapsed() > Duration::from_secs(10) {
            let log = env.log();
            let events: Vec<&str> = log.lines().filter(|l| l.contains("macos")).collect();
            panic!("the drag didn't land; badge events:\n{}", events.join("\n"));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
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

/// Size > 32 from the menu-bar item with posted input: a click opens the
/// menu, the arrow keys walk it, Return chooses. The badge shrinks and the
/// choice lands in this host's table (SPEC §8.6).
fn menu_choice(env: &Env, pid: u32, rect: [i32; 4], scale: f64) {
    let item = find_menu_bar_item(env);
    let b = item.bounds;
    let at = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    post_mouse(CGEventType::MouseMoved, at);
    post_mouse(CGEventType::LeftMouseDown, at);
    post_mouse(CGEventType::LeftMouseUp, at);
    std::thread::sleep(Duration::from_millis(500));
    // Move, Size, into the submenu (its first item, 32), choose.
    for key in [125, 125, 124, 36] {
        post_key(key);
        std::thread::sleep(Duration::from_millis(150));
    }
    let start = Instant::now();
    loop {
        let now = env.badge_rect();
        if now.is_some_and(|r| r[2] == rect[2] / 2) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the menu choice didn't land: badge {now:?}, was {rect:?}\nwindows {:?}\nbadge log:\n{}",
            windows_of(pid),
            env.log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let config = env.config();
    assert!(config.contains("[hosts.owl]\nsize = 32\n"), "{config}");
    eprintln!("menu: Size > 32 resized the badge and saved size = 32 (scale {scale})");
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

    // The menu-bar item: gone with `tray = false`, back without it.
    let item = find_menu_bar_item(&env);
    eprintln!("menu-bar item: {item:?}");
    let doctor = env.ok(&["doctor"]);
    assert!(doctor.contains("NSStatusItem"), "{doctor}");

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
        menu_choice(&env, badge.0.id(), rect, scale);
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
        // The `.lock` file stays by design (SPEC §10.4); only the socket goes.
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
                .count()
        })
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

/// Static mode (SPEC §7.2): with nothing to animate and nobody asking, the
/// loop never wakes, menu-bar item included. (The process still shows a
/// few wakeups a second from AppKit's own threads; `make power-check`
/// reports them.)
#[test]
#[ignore = "needs a logged-in macOS desktop; run with `make test-macos-display`"]
fn static_mode_loop_sleeps() {
    let mut env = Env::new("macos-static");
    env.set("TMPDIR", env.runtime().to_string_lossy());
    env.write_config("animation = \"static\"\n");
    let mut badge = env.run_badge();
    wait_for("the badge to be placed", || {
        if let Some(code) = badge.exited() {
            panic!("the badge exited ({code}):\n{}", env.log());
        }
        env.badge_rect()
    });
    std::thread::sleep(Duration::from_secs(5));
    let woke = |log: &str| log.matches("macos: woke").count();
    let before = woke(&env.log());
    assert!(before > 0, "no wakeup lines with --verbose:\n{}", env.log());
    std::thread::sleep(Duration::from_secs(15));
    let log = env.log();
    assert_eq!(woke(&log), before, "the loop woke in static mode:\n{log}");
    eprintln!("static mode: no loop wakeup in 15 s");
}

const LABEL: &str = "io.github.nealhardesty.pawprint";

/// `launchctl print gui/<uid>/<LABEL>`, if launchd has the agent.
fn launchctl_print() -> Option<String> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let out = std::process::Command::new("/bin/launchctl")
        .arg("print")
        .arg(format!("gui/{uid}/{LABEL}"))
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `autostart on|off|status`: the LaunchAgent file and launchd agree
/// (SPEC §11.2). `on` loads the agent, which starts a badge; `off` boots it
/// out, which stops that badge, and removes the file.
#[test]
#[ignore = "needs a logged-in macOS desktop; run with `make test-macos-display`"]
fn autostart_round_trip() {
    if launchctl_print().is_some() {
        eprintln!("SKIP autostart: {LABEL} is already loaded for this user; not touching it");
        return;
    }
    let env = Env::new("macos-autostart");
    let plist = env
        .home()
        .join(format!("Library/LaunchAgents/{LABEL}.plist"));
    assert!(env.ok(&["autostart", "status"]).starts_with("off"));

    let on = env.ok(&["autostart", "on"]);
    eprintln!("{on}");
    let text = std::fs::read_to_string(&plist).unwrap();
    for want in [
        format!(
            "<string>{}</string>\n        <string>run</string>",
            env!("CARGO_BIN_EXE_pawprint")
        ),
        "<key>LimitLoadToSessionType</key>\n    <string>Aqua</string>".into(),
        "<key>SuccessfulExit</key>\n        <false/>".into(),
        "<key>RunAtLoad</key>\n    <true/>".into(),
        "<key>ProcessType</key>\n    <string>Interactive</string>".into(),
    ] {
        assert!(text.contains(&want), "no {want:?} in\n{text}");
    }
    let lint = std::process::Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(&plist)
        .output()
        .unwrap();
    assert!(lint.status.success(), "{lint:?}");

    assert!(launchctl_print().is_some(), "autostart on loads the agent");
    let status = env.ok(&["autostart", "status"]);
    assert!(
        status.starts_with("on: ") && !status.contains("not loaded"),
        "{status}"
    );
    assert!(env.ok(&["doctor"]).contains("run at login"));
    // The agent runs `pawprint run`, which stays up: no badge ran before.
    let pid = wait_for("launchd to start the badge", || {
        launchctl_print()?
            .lines()
            .find_map(|l| l.trim().strip_prefix("pid = ")?.parse::<i32>().ok())
    });
    eprintln!("autostart: launchd started the badge as pid {pid}");

    let off = env.ok(&["autostart", "off"]);
    assert!(off.contains("Autostart is off"), "{off}");
    assert!(
        launchctl_print().is_none(),
        "autostart off boots the agent out"
    );
    assert!(!plist.exists());
    assert!(env.ok(&["autostart", "status"]).starts_with("off"));
    wait_for("the launchd badge to stop", || {
        // SAFETY: signal 0 only checks that the process exists.
        (unsafe { libc::kill(pid, 0) } != 0).then_some(())
    });
    assert!(env.ok(&["autostart", "off"]).contains("already off"));
}

/// `pawprint preview`: a normal titled window (window layer 0) of a
/// regular app at the front, which Esc and the close button end.
#[test]
#[ignore = "needs a logged-in macOS desktop; run with `make test-macos-display`"]
fn preview_window() {
    let env = Env::new("macos-preview");
    let items = menu_bar_items().len();
    let open = || {
        let mut p = env.spawn(env!("CARGO_BIN_EXE_pawprint"), &["preview", "owl"]);
        let pid = p.0.id();
        let w = wait_for("the preview window", || {
            if let Some(code) = p.exited() {
                panic!("preview exited ({code}):\n{}", env.log());
            }
            window_of(pid)
        });
        (p, w)
    };
    let (mut p, w) = open();
    let pid = p.0.id();
    eprintln!("preview window: {w:?}");
    assert_eq!(w.layer, 0, "a normal window, not the overlay level");
    match &w.name {
        Some(name) => assert_eq!(name, "PAWPRINT preview"),
        None => eprintln!("SKIP preview title: the window server withholds window names"),
    }
    // 128 px and a 16 px margin on each side, plus a title bar, at scale 1
    // (the runner's display).
    let content = 128.0 + 2.0 * 16.0;
    assert!(
        (w.bounds[2] - content).abs() < 0.01 && w.bounds[3] > content,
        "preview frame {:?}",
        w.bounds
    );
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32)
        .expect("the preview is a running application");
    assert_eq!(
        app.activationPolicy(),
        NSApplicationActivationPolicy::Regular
    );
    assert!(
        menu_bar_items().len() <= items,
        "no menu-bar item of its own"
    );

    if !CGPreflightPostEventAccess() {
        eprintln!("SKIP preview Esc and close: this process may not post events");
        return;
    }
    wait_for("the preview to be in front", || in_front(pid).then_some(()));
    post_key(53); // escape
    let code = wait_for("Esc to end the preview", || p.exited());
    assert!(code.success(), "exit {code}");

    // The close button, pressed through Accessibility (its place in the
    // title bar differs between macOS releases, and a miss drags the
    // window).
    let (mut p, _) = open();
    let script = format!(
        "tell application \"System Events\" to click (first button whose subrole is \
         \"AXCloseButton\") of window 1 of (first process whose unix id is {})",
        p.0.id()
    );
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .unwrap();
    if !out.status.success() {
        eprintln!(
            "SKIP preview close button: osascript failed (Accessibility for System Events?): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return;
    }
    let code = wait_for("the close button to end the preview", || p.exited());
    assert!(code.success(), "exit {code}");
    eprintln!("preview: Esc and the close button each end it");
}
