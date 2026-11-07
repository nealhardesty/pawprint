//! The Windows backend on a real desktop (SPEC §11.1): window styles,
//! placement in the work area, topmost, click-through, move mode with a real
//! drag, work-area changes, IPC over the named pipe, single instance,
//! detached start through `pawprintw.exe`, the pixels on screen, the tray
//! icon and its menu, autostart's Run value, and the preview window. Run
//! with `make test-windows-display`; CI runs it on windows-latest.
//!
//! It runs in your own session: it covers the primary work area with a
//! solid window for a few seconds, docks a bar at the top of the screen for
//! a moment, opens the tray menu, and restores your state file, config
//! file, and autostart value afterwards.

#![cfg(windows)]

mod common;

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{Env, Image, assert_badge_at, wait_for};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{
    ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA, NIM_MODIFY,
    NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, SHAppBarMessage, Shell_NotifyIconGetRect,
    Shell_NotifyIconW,
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

/// Puts one of this user's files (`pawprint paths` names it) back as it
/// was.
struct FileGuard {
    path: std::path::PathBuf,
    saved: Option<Vec<u8>>,
}

impl FileGuard {
    fn new(env: &Env, name: &str) -> FileGuard {
        let paths = env.ok(&["paths"]);
        let line = paths
            .lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("a {name} path in {paths}"));
        let path = std::path::PathBuf::from(line[name.len()..].trim());
        let saved = std::fs::read(&path).ok();
        FileGuard { path, saved }
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }
}

impl Drop for FileGuard {
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
    assert_no_badge(&env);
    let state = FileGuard::new(&env, "state");
    let config = FileGuard::new(&env, "config");
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

    // The tray icon is registered on the badge window (SPEC §11.1.5).
    assert_tray_icon(hwnd);

    // Points inside a 64-px badge (scaled): the label strip and the body.
    let at = |px: i32, py: i32| (x + px * w / 64, y + py * h / 64);
    let label = at(32, 60);
    let body = at(32, 32);
    let expected = env.expected(64);
    // Inside the edge, where move mode draws its dashed outline (SPEC §6.3).
    let corner = (2..62)
        .flat_map(|py| (2..62).map(move |px| (px, py)))
        .find(|&(px, py)| expected.px(px as u32, py as u32)[3] == 0)
        .map(|(px, py)| at(px, py))
        .expect("a transparent pixel");

    let desktop = capture(rect);
    match desktop.clone() {
        None => println!(
            "SKIP pixel, click, drag, key, and tray menu checks: this session has no \
             visible desktop (screen capture shows nothing)"
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

    // The tray menu: Size ▸ 128 resizes the badge and is saved in this
    // host's table (SPEC §8.6); `unset` brings it back.
    if desktop.is_some() {
        choose_size_128(hwnd, work);
        let [bx, by, bw, _] = wait_for("the badge at size 128", || {
            env.badge_rect().filter(|r| r[2] == 2 * w)
        });
        assert_eq!((bx + bw, by), (x + w, y), "still top-right");
        let saved = config.text();
        assert!(
            saved.contains("size = 128")
                && (saved.contains("[hosts.owl]") || saved.contains("[hosts.\"owl\"]")),
            "the choice is saved in this host's table:\n{saved}"
        );
        env.ok(&["unset", "size"]);
        env.wait_rect(rect);
    }

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

fn assert_no_badge(env: &Env) {
    assert_eq!(
        env.pawprint(&["status"]).status.code(),
        Some(4),
        "a badge is already running in this session; `pawprint quit` it first"
    );
}

/// The tray's callback message and icon id (`backend::windows`).
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;

/// The notification area knows the badge's icon: its rectangle, or, when
/// the icon sits in the hidden overflow, a no-op modify that only succeeds
/// for a registered icon.
fn assert_tray_icon(hwnd: HWND) {
    let id = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    let mut r = RECT::default();
    let hr = unsafe { Shell_NotifyIconGetRect(&id, &mut r) };
    if hr == 0 {
        println!(
            "tray icon at {:?}",
            [r.left, r.top, r.right - r.left, r.bottom - r.top]
        );
        return;
    }
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    };
    let known = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0;
    println!("tray icon: Shell_NotifyIconGetRect {hr:#x}; registered: {known}");
    assert!(known, "the badge has no notification area icon");
}

/// The visible popup menu of `pid`'s thread other than `except`, and its
/// HMENU.
fn popup_menu(pid: u32, except: HMENU) -> Option<(HWND, HMENU)> {
    let class = wide("#32768");
    let mut after = null_mut();
    loop {
        let w = unsafe { FindWindowExW(null_mut(), after, class.as_ptr(), null()) };
        if w.is_null() {
            return None;
        }
        after = w;
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(w, &mut owner) };
        let menu = unsafe { SendMessageW(w, MN_GETHMENU, 0, 0) } as HMENU;
        if owner == pid && unsafe { IsWindowVisible(w) } != 0 && !menu.is_null() && menu != except {
            return Some((w, menu));
        }
    }
}

fn menu_label(menu: HMENU, i: u32) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetMenuStringW(menu, i, buf.as_mut_ptr(), 64, MF_BYPOSITION) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Click the middle of item `i` of an open popup menu.
fn click_menu_item(menu: HMENU, i: u32) {
    let mut r = RECT::default();
    assert_ne!(
        unsafe { GetMenuItemRect(null_mut(), menu, i, &mut r) },
        0,
        "menu item {i} has a rectangle"
    );
    click((r.left + r.right) / 2, (r.top + r.bottom) / 2);
}

/// Open the tray menu the way the shell does for a right click on the icon
/// (NOTIFYICON_VERSION_4), check it mirrors the core's model, and choose
/// Size ▸ 128 with real clicks.
fn choose_size_128(hwnd: HWND, work: RECT) {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let (x, y) = (work.left + 100, work.top + 100);
    unsafe {
        // The badge sets the menu's owner as the foreground window.
        AllowSetForegroundWindow(pid);
        let anchor = ((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as WPARAM;
        PostMessageW(
            hwnd,
            WM_TRAY,
            anchor,
            ((TRAY_ID << 16) | WM_CONTEXTMENU) as LPARAM,
        );
    }
    let (_, menu) = wait_for("the tray menu", || popup_menu(pid, null_mut()));
    let labels: Vec<String> = (0..unsafe { GetMenuItemCount(menu) } as u32)
        .map(|i| menu_label(menu, i))
        .collect();
    assert_eq!(
        labels,
        [
            "Move",
            "Size",
            "Animation",
            "Reset position",
            "Reload config",
            "Quit"
        ]
    );
    let size = unsafe { GetSubMenu(menu, 1) };
    let checked = |i| unsafe { GetMenuState(size, i, MF_BYPOSITION) } & MF_CHECKED != 0;
    assert_eq!(
        (menu_label(size, 1), checked(0), checked(1), checked(2)),
        ("64".into(), false, true, false),
        "Size ▸ 64 is checked"
    );
    click_menu_item(menu, 1);
    let (_, sub) = wait_for("the Size submenu", || popup_menu(pid, menu));
    assert_eq!(menu_label(sub, 2), "128");
    click_menu_item(sub, 2);
    wait_for("the menu to close", || {
        popup_menu(pid, null_mut()).is_none().then_some(())
    });
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// HKCU Run's `pawprint` value, read without PAWPRINT.
fn run_value() -> Option<String> {
    let (key, value) = (wide(RUN_KEY), wide("pawprint"));
    let mut buf = [0u16; 1024];
    let mut bytes = size_of_val(&buf) as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    (code == ERROR_SUCCESS).then(|| {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    })
}

/// Puts this user's autostart value back as it was.
struct RunValueGuard(Option<String>);

impl Drop for RunValueGuard {
    fn drop(&mut self) {
        let (key, value) = (wide(RUN_KEY), wide("pawprint"));
        unsafe {
            match &self.0 {
                Some(v) => {
                    let data = wide(v);
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        key.as_ptr(),
                        value.as_ptr(),
                        REG_SZ,
                        data.as_ptr().cast(),
                        (data.len() * 2) as u32,
                    );
                }
                None => {
                    RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr());
                }
            }
        }
    }
}

#[test]
#[ignore = "writes this user's HKCU Run value (restored after); run `make test-windows-display`"]
fn autostart_writes_the_run_value() {
    let env = Env::new("windows-autostart");
    let _guard = RunValueGuard(run_value());
    let badge =
        std::path::Path::new(env!("CARGO_BIN_EXE_pawprint")).with_file_name("pawprintw.exe");
    let want = format!("\"{}\" run", badge.display());

    let out = env.ok(&["autostart", "on"]);
    assert_eq!(run_value().as_deref(), Some(want.as_str()), "{out}");
    let status = env.ok(&["autostart", "status"]);
    assert!(
        status.starts_with("on: ") && status.contains(&want),
        "{status}"
    );
    assert!(status.contains(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run\pawprint"));

    let out = env.ok(&["autostart", "off"]);
    assert!(out.contains("Autostart is off"), "{out}");
    assert_eq!(run_value(), None);
    assert!(env.ok(&["autostart", "status"]).starts_with("off"));
    assert!(env.ok(&["autostart", "off"]).contains("already off"));
}

fn preview_window() -> Option<HWND> {
    let class = wide("PawprintPreview");
    let hwnd = unsafe { FindWindowW(class.as_ptr(), null()) };
    (!hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0).then_some(hwnd)
}

#[test]
#[ignore = "needs a Windows desktop session; run `make test-windows-display`"]
fn preview_is_a_normal_window() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let env = Env::new("windows-preview");
    let (_, scale) = primary();
    let s = |n: i32| (f64::from(n) * scale).round() as i32;
    let exe = env!("CARGO_BIN_EXE_pawprintw");

    // A titled, closable window with a taskbar button: none of the badge's
    // overlay styles.
    let mut preview = env.spawn(exe, &["preview", "owl", "--size", "64"]);
    let hwnd = wait_for("the preview window", preview_window);
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    assert!(
        style & WS_CAPTION == WS_CAPTION && style & WS_SYSMENU != 0,
        "{style:#x}"
    );
    assert_eq!(style & WS_POPUP, 0, "{style:#x}");
    let ex = ex_style(hwnd);
    for (flag, name) in [
        (WS_EX_LAYERED, "WS_EX_LAYERED"),
        (WS_EX_TRANSPARENT, "WS_EX_TRANSPARENT"),
        (WS_EX_TOPMOST, "WS_EX_TOPMOST"),
        (WS_EX_TOOLWINDOW, "WS_EX_TOOLWINDOW"),
        (WS_EX_NOACTIVATE, "WS_EX_NOACTIVATE"),
    ] {
        assert_eq!(ex & flag, 0, "{name} set on the preview: {ex:#x}");
    }
    let mut title = [0u16; 64];
    let n = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), 64) };
    assert_eq!(
        String::from_utf16_lossy(&title[..n as usize]),
        "PAWPRINT preview"
    );

    // The client area is the badge plus the margin, on the backdrop.
    let mut client = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
        ClientToScreen(hwnd, &mut origin);
    }
    // Windows may widen a captioned window to its minimum width; the badge
    // then sits centered on the backdrop.
    assert_eq!(client.bottom, s(64 + 32), "the badge plus the margin");
    assert!(client.right >= s(64 + 32), "client width {}", client.right);
    unsafe { SetForegroundWindow(hwnd) };
    settle();
    match capture([origin.x, origin.y, client.right, client.bottom]) {
        None => println!("SKIP preview pixels: no visible desktop"),
        Some(shot) => assert_eq!(shot.px(1, 1)[..3], [0x5a, 0x60, 0x6b], "the slate backdrop"),
    }

    // Closing the window ends the preview.
    close(hwnd);
    let status = wait_for("the preview to exit", || preview.exited());
    assert!(status.success(), "{status:?}");
    assert!(preview_window().is_none());

    // So does Esc.
    let mut preview = env.spawn(exe, &["preview", "owl", "--size", "64"]);
    let hwnd = wait_for("the preview window", preview_window);
    unsafe { PostMessageW(hwnd, WM_KEYDOWN, VK_ESCAPE as WPARAM, 0) };
    let status = wait_for("the preview to exit on Esc", || preview.exited());
    assert!(status.success(), "{status:?}");
}
