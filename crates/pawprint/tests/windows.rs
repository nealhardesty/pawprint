//! The Windows backend on a real desktop (SPEC §11.1): window styles,
//! placement in the work area, topmost, click-through, move mode with a real
//! drag, work-area changes, IPC over the named pipe, single instance,
//! detached start through `pawprintw.exe`, and the pixels on screen. Run with
//! `make test-windows-display`; CI runs it on windows-latest.
//!
//! It runs in your own session: it covers the primary work area with a
//! solid window for a few seconds, docks a bar at the top of the screen for
//! a moment, and restores your state file afterwards.

#![cfg(windows)]

mod common;

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{Env, Image, assert_badge_at, wait_for};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{
    ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA, SHAppBarMessage,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const BACKGROUND: [u8; 3] = [0x33, 0x66, 0x99];
const BG_COLORREF: u32 = 0x0099_6633; // 0x00BBGGRR

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Create a window on a thread of its own that keeps dispatching messages,
/// so the test's windows never hold up the shell's synchronous broadcasts
/// while the test thread waits on a child process. Close it with `close`.
fn ui_window(make: impl FnOnce() -> HWND + Send + 'static) -> HWND {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        tx.send(make() as isize).unwrap();
        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
            unsafe { DispatchMessageW(&msg) };
        }
    });
    rx.recv().unwrap() as HWND
}

fn close(hwnd: HWND) {
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
}

/// A popup window of its own class.
fn popup(class: &str, ex: u32, r: RECT, proc: WNDPROC, brush: HBRUSH) -> HWND {
    let class = wide(class);
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: proc,
            lpszClassName: class.as_ptr(),
            hbrBackground: brush,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            ex,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!hwnd.is_null(), "cannot create a test window");
        hwnd
    }
}

fn settle() {
    std::thread::sleep(Duration::from_millis(300));
}

fn badge_window() -> Option<HWND> {
    let class = wide("PawprintBadge");
    let hwnd = unsafe { FindWindowW(class.as_ptr(), null()) };
    (!hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0).then_some(hwnd)
}

fn ex_style(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }
}

fn window_rect(hwnd: HWND) -> [i32; 4] {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) };
    [r.left, r.top, r.right - r.left, r.bottom - r.top]
}

fn window_at(x: i32, y: i32) -> HWND {
    unsafe { WindowFromPoint(POINT { x, y }) }
}

/// The primary monitor's work area and scale.
fn primary() -> (RECT, f64) {
    unsafe {
        let m = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(m, &mut info);
        let (mut dpi, mut _y) = (96, 96);
        GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
        (info.rcWork, f64::from(dpi) / 96.0)
    }
}

/// An app bar docked to the top edge of the primary display, like a second
/// taskbar: the shell takes its height out of the work area and broadcasts
/// the change. Removed on drop, even after a failure.
struct AppBar(APPBARDATA);

impl AppBar {
    fn top(work: RECT, height: i32) -> AppBar {
        let hwnd = ui_window(|| {
            popup(
                "PawprintTestAppBar",
                WS_EX_TOOLWINDOW,
                RECT::default(),
                Some(DefWindowProcW),
                null_mut(),
            )
        });
        let mut bar = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: WM_APP,
            uEdge: ABE_TOP,
            rc: RECT {
                bottom: work.top + height,
                ..work
            },
            lParam: 0,
        };
        unsafe {
            assert_ne!(SHAppBarMessage(ABM_NEW, &mut bar), 0, "ABM_NEW");
            SHAppBarMessage(ABM_QUERYPOS, &mut bar);
            bar.rc.bottom = bar.rc.top + height;
            SHAppBarMessage(ABM_SETPOS, &mut bar);
        }
        let r = bar.rc;
        println!("app bar at {:?}", [r.left, r.top, r.right, r.bottom]);
        AppBar(bar)
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut self.0) };
        close(self.0.hWnd);
    }
}

/// The screen inside `r`, or `None` when the session has no visible desktop
/// to capture. `CAPTUREBLT` includes layered windows.
fn capture(r: [i32; 4]) -> Option<Image> {
    let [x, y, w, h] = r;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let ok = unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let old = SelectObject(mem, bitmap);
        let ok = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
        if ok {
            let src = std::slice::from_raw_parts(bits.cast::<u8>(), rgba.len());
            for (d, s) in rgba.chunks_mut(4).zip(src.chunks(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], 255]);
            }
        }
        SelectObject(mem, old);
        DeleteObject(bitmap);
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        ok
    };
    let img = Image {
        width: w as u32,
        height: h as u32,
        rgba,
    };
    // A session without a visible desktop captures one flat color.
    (ok && img.colors([0, 0, w, h]).len() > 1).then_some(img)
}

static CLICKED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn background_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_LBUTTONDOWN {
        CLICKED.store(true, Ordering::SeqCst);
    }
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// A plain, solid, normal (not topmost) window covering `r`, shown after
/// the badge so it would cover the badge if the badge were not topmost.
fn background(r: RECT) -> HWND {
    ui_window(move || unsafe {
        let brush = CreateSolidBrush(BG_COLORREF);
        let hwnd = popup("PawprintTestBackground", 0, r, Some(background_proc), brush);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        UpdateWindow(hwnd);
        hwnd
    })
}

fn send(input: INPUT) {
    unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
}

fn button(down: bool) {
    let mut input = INPUT {
        r#type: INPUT_MOUSE,
        ..Default::default()
    };
    input.Anonymous.mi.dwFlags = if down {
        MOUSEEVENTF_LEFTDOWN
    } else {
        MOUSEEVENTF_LEFTUP
    };
    send(input);
    settle();
}

fn pointer_to(x: i32, y: i32) {
    unsafe { SetCursorPos(x, y) };
    settle();
}

fn click(x: i32, y: i32) {
    pointer_to(x, y);
    button(true);
    button(false);
}

fn drag(from: (i32, i32), to: (i32, i32)) {
    pointer_to(from.0, from.1);
    button(true);
    for i in 1..=10 {
        pointer_to(
            from.0 + (to.0 - from.0) * i / 10,
            from.1 + (to.1 - from.1) * i / 10,
        );
    }
    button(false);
}

fn key(vk: VIRTUAL_KEY) {
    for up in [false, true] {
        let mut input = INPUT {
            r#type: INPUT_KEYBOARD,
            ..Default::default()
        };
        input.Anonymous.ki.wVk = vk;
        input.Anonymous.ki.dwFlags = if up { KEYEVENTF_KEYUP } else { 0 };
        send(input);
    }
    settle();
}

/// Puts this user's state file back as it was.
struct StateGuard {
    path: std::path::PathBuf,
    saved: Option<Vec<u8>>,
}

impl StateGuard {
    fn new(env: &Env) -> StateGuard {
        let paths = env.ok(&["paths"]);
        let line = paths
            .lines()
            .find(|l| l.starts_with("state"))
            .expect("a state path");
        let path = std::path::PathBuf::from(line["state".len()..].trim());
        let saved = std::fs::read(&path).ok();
        StateGuard { path, saved }
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }
}

impl Drop for StateGuard {
    fn drop(&mut self) {
        let _ = match &self.saved {
            Some(bytes) => std::fs::write(&self.path, bytes),
            None => std::fs::remove_file(&self.path),
        };
    }
}

#[test]
#[ignore = "needs a Windows desktop session; run `make test-windows-display`"]
fn layered_topmost_click_through_badge() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let env = Env::new("windows");
    // A hung Win32 call would otherwise stall CI until the job times out.
    let log = env.dir.join("pawprintw.exe.log");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(120));
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let tail = &text[text.len().saturating_sub(3000)..];
        eprintln!("watchdog: the test hung for 2 minutes; badge log:\n{tail}");
        std::process::exit(101);
    });
    assert_eq!(
        env.pawprint(&["status"]).status.code(),
        Some(4),
        "a badge is already running in this session; `pawprint quit` it first"
    );
    let state = StateGuard::new(&env);
    let (work, scale) = primary();
    let s = |n: i32| (f64::from(n) * scale).round() as i32;

    // The GUI-subsystem badge, its stderr redirected to a file.
    let mut badge = env.spawn(env!("CARGO_BIN_EXE_pawprintw"), &["run", "--verbose"]);
    let hwnd = wait_for("the badge window", badge_window);
    let rect = wait_for("the badge to be placed", || env.badge_rect());
    let [x, y, w, h] = rect;
    let area = [work.left, work.top, work.right, work.bottom];
    println!("badge {rect:?}, work area (l, t, r, b) {area:?}, scale {scale}");
    let bg = background(work);
    settle();

    // Top-right of the work area, 16 logical px in (SPEC §6.1).
    assert_eq!(
        (x + w, y),
        (work.right - s(16), work.top + s(16)),
        "top-right of the work area"
    );
    assert_eq!(window_rect(hwnd), rect, "the window is where status says");

    // SPEC §11.1.1 styles.
    let ex = ex_style(hwnd);
    for (flag, name) in [
        (WS_EX_LAYERED, "WS_EX_LAYERED"),
        (WS_EX_TRANSPARENT, "WS_EX_TRANSPARENT"),
        (WS_EX_TOPMOST, "WS_EX_TOPMOST"),
        (
            WS_EX_TOOLWINDOW,
            "WS_EX_TOOLWINDOW (not in the taskbar or Alt+Tab)",
        ),
        (WS_EX_NOACTIVATE, "WS_EX_NOACTIVATE"),
    ] {
        assert!(ex & flag != 0, "{name} missing: {ex:#x}");
    }
    assert_eq!(
        ex & WS_EX_APPWINDOW,
        0,
        "WS_EX_APPWINDOW would add a taskbar button"
    );
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    assert!(
        style & WS_POPUP != 0 && style & WS_CAPTION == 0,
        "{style:#x}"
    );
    assert_ne!(
        unsafe { GetForegroundWindow() },
        hwnd,
        "the badge never takes the focus"
    );

    // IPC and single instance.
    let second = env.pawprint(&["run"]);
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("already running"));
    let status = env.ok(&["status"]);
    assert!(status.contains("windows"), "{status}");

    // Points inside a 64-px badge (scaled): the label strip and the body.
    let at = |px: i32, py: i32| (x + px * w / 64, y + py * h / 64);
    let label = at(32, 60);
    let body = at(32, 32);
    let expected = env.expected(64);
    let corner = (0..64)
        .flat_map(|py| (0..64).map(move |px| (px, py)))
        .find(|&(px, py)| expected.px(px as u32, py as u32)[3] == 0)
        .map(|(px, py)| at(px, py))
        .expect("a transparent pixel");

    match capture(rect) {
        None => println!(
            "SKIP pixel, click, drag, and key checks: this session has no visible \
             desktop (screen capture shows nothing)"
        ),
        Some(screen) => {
            if scale == 1.0 {
                // Above the newer normal window, transparent around the art.
                assert_badge_at(&screen, &expected, 0, 0, BACKGROUND);
            } else {
                println!("SKIP exact pixels: display scale {scale} (golden images are 1x)");
            }

            // Click-through: a click on the label reaches the window below.
            assert_eq!(
                window_at(label.0, label.1),
                bg,
                "hit testing skips the badge"
            );
            CLICKED.store(false, Ordering::SeqCst);
            click(label.0, label.1);
            assert!(CLICKED.load(Ordering::SeqCst), "the click passes through");

            // Move mode takes the pointer on the art; transparent pixels
            // still pass it through (per-pixel hit testing).
            env.ok(&["move"]);
            assert_eq!(ex_style(hwnd) & WS_EX_TRANSPARENT, 0, "move mode");
            assert_eq!(
                window_at(body.0, body.1),
                hwnd,
                "move mode takes the pointer"
            );
            assert_eq!(
                window_at(corner.0, corner.1),
                bg,
                "transparent pixels pass the pointer even in move mode"
            );
            drag(body, (body.0 - s(300), body.1 + s(200)));
            let moved = [x - s(300), y + s(200), w, h];
            env.wait_rect(moved);
            assert_eq!(window_rect(hwnd), moved);
            assert!(
                ex_style(hwnd) & WS_EX_TRANSPARENT != 0,
                "click-through again"
            );
            let saved = state.text();
            assert!(
                saved.contains("anchor") && saved.contains("offset"),
                "{saved}"
            );
            if scale == 1.0 {
                let screen = capture(moved).expect("a capture");
                assert_badge_at(&screen, &expected, 0, 0, BACKGROUND);
            }

            // Arrow keys, where Windows gives the badge the keyboard.
            env.ok(&["move"]);
            settle();
            if unsafe { GetForegroundWindow() } == hwnd {
                key(VK_LEFT);
                key(VK_RETURN);
                env.wait_rect([moved[0] - s(1), moved[1], w, h]);
                println!("keys: the badge took the keyboard in move mode");
            } else {
                println!(
                    "SKIP keys: Windows did not give the badge the keyboard here \
                     (no foreground rights); ending move mode with a click"
                );
                let (bx, by) = (body.0 - s(300), body.1 + s(200));
                click(bx, by);
            }
            wait_for("move mode to end", || {
                (ex_style(hwnd) & WS_EX_TRANSPARENT != 0).then_some(())
            });
        }
    }

    env.ok(&["reset-position"]);
    env.wait_rect(rect);

    // A bar docked at the top shrinks the work area: the badge moves down
    // with it, and back when the bar goes.
    {
        let _bar = AppBar::top(work, s(40));
        let (now, _) = primary();
        assert_eq!(now.top, work.top + s(40), "the app bar reserved its space");
        env.wait_rect([x, y + s(40), w, h]);
    }
    env.wait_rect(rect);

    env.ok(&["quit"]);
    wait_for("the badge to exit", || badge.exited());
    assert!(badge_window().is_none(), "window gone");
    assert_eq!(env.pawprint(&["status"]).status.code(), Some(4));

    // Bare `pawprint` starts `pawprintw.exe run` detached.
    let out = env.ok(&[]);
    assert!(out.contains("PAWPRINT started"), "{out}");
    wait_for("the detached badge window", badge_window);
    env.ok(&["quit"]);
    wait_for("the detached badge to exit", || {
        badge_window().is_none().then_some(())
    });
    close(bg);
}
