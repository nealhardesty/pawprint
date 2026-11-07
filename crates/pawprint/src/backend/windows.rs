//! Windows backend (SPEC §11.1): a layered, topmost, click-through tool
//! window, drawn with `UpdateLayeredWindow` from the shared premultiplied
//! BGRA frame.
//!
//! The window procedure only records what happened, as core events, in a
//! thread-local queue, and the loop hands them to the shell. Win32 calls
//! such as `SetWindowPos` send messages synchronously, so this keeps the
//! shell from being re-entered while it applies an effect.

use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};
use std::sync::mpsc;

use pawprint_core::badge::{Command, Event};
use pawprint_core::config::Backend;
use pawprint_core::geom::{PhysicalRect, Point};
use pawprint_core::movemode::{Key, KeyCode};
use pawprint_core::render::Frame;
use pawprint_core::surface::{DisplayInfo, Surface, select_display};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetMonitorInfoW, HBITMAP, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::BOOL;

use crate::doctor::Report;
use crate::shell::Shell;
use crate::{CmdResult, exit, ipc};

const CLASS: &str = "PawprintBadge";

/// Posted by the IPC thread to wake the loop. The M4 tray will add its
/// icon's callback message next to this one and turn menu choices into
/// `Event::Command(..)` in `wndproc`; the loop needs nothing else.
const WM_WAKE: u32 = WM_APP;

/// `WS_EX_TRANSPARENT` is cleared in move mode, so the badge takes the
/// pointer. A click never activates it (`WS_EX_NOACTIVATE`).
const EX_STYLE: WINDOW_EX_STYLE =
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;

thread_local! {
    /// Events `wndproc` recorded for the loop.
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
    static INTERACTIVE: Cell<bool> = const { Cell::new(false) };
    static DRAGGING: Cell<bool> = const { Cell::new(false) };
}

fn push(event: Event) {
    EVENTS.with_borrow_mut(|events| events.push(event));
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Per-monitor DPI awareness v2 (SPEC §11.1.2), so every coordinate is in
/// physical pixels. False on Windows older than 10 1703.
fn dpi_aware() -> bool {
    // SAFETY: no preconditions; fails harmlessly if already set.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 }
}

pub fn run(mut shell: Shell, listener: ipc::Listener) -> CmdResult {
    if !dpi_aware() {
        log::warn!("per-monitor DPI awareness unavailable; positions may be scaled");
    }
    let configured = shell.configured_backend();
    if !matches!(configured, Backend::Auto | Backend::Windows) {
        log::warn!("backend = \"{configured}\" does not exist on Windows; using windows");
    }
    shell.set_backend("windows");
    let mut window = Window::create().map_err(|e| {
        (
            exit::NO_SESSION,
            format!("cannot create the badge window: {e}. Run `pawprint doctor`."),
        )
    })?;

    let (tx, rx) = mpsc::channel::<ipc::Incoming>();
    // HWND is a pointer and not Send; the IPC thread only posts to it.
    let hwnd = window.hwnd as isize;
    ipc::serve(listener, move |incoming| {
        // SAFETY: posting to a window that may be gone just fails.
        tx.send(incoming).is_ok() && unsafe { PostMessageW(hwnd as HWND, WM_WAKE, 0, 0) } != 0
    });

    shell.start(&mut window);
    loop {
        pump();
        for event in EVENTS.take() {
            shell.handle(&mut window, event);
        }
        while let Ok(incoming) = rx.try_recv() {
            shell.request(&mut window, incoming);
        }
        shell.tick(&mut window);
        if shell.done() {
            return Ok(());
        }
        // Applying effects can send messages that record new events.
        let wait = if EVENTS.with_borrow(|e| !e.is_empty()) {
            0
        } else {
            shell.timeout().map_or(INFINITE, |d| {
                u32::try_from(d.as_micros().div_ceil(1000)).unwrap_or(INFINITE - 1)
            })
        };
        // SAFETY: no handles; wakes on any input or posted message.
        unsafe { MsgWaitForMultipleObjects(0, null(), 0, wait, QS_ALLINPUT) };
    }
}

/// Dispatch every queued message to `wndproc`.
fn pump() {
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid MSG for each call.
    while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
        if msg.message == WM_QUIT {
            push(Event::Command(Command::Quit));
        }
        unsafe { DispatchMessageW(&msg) };
    }
}

/// The pointer in physical desktop coordinates. The window moves under the
/// pointer while dragging, so window-relative coordinates would run away.
fn cursor() -> Point {
    let mut p = POINT::default();
    // SAFETY: writes into `p`.
    unsafe { GetCursorPos(&mut p) };
    Point::new(p.x, p.y)
}

fn key_code(vk: WPARAM) -> Option<KeyCode> {
    Some(match u16::try_from(vk).ok()? {
        VK_LEFT => KeyCode::Left,
        VK_RIGHT => KeyCode::Right,
        VK_UP => KeyCode::Up,
        VK_DOWN => KeyCode::Down,
        VK_RETURN => KeyCode::Enter,
        VK_ESCAPE => KeyCode::Escape,
        _ => return None,
    })
}

/// A display, resolution, scale, or work area changed: stay on top (SPEC
/// §11.1.3) and report the new set; the shell drops repeats.
fn displays_changed(hwnd: HWND, why: &str) {
    log::debug!("windows: {why}");
    // SAFETY: a valid window; changes only the z-order.
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    push(Event::DisplaysChanged(monitors()));
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let interactive = INTERACTIVE.get();
    // SAFETY (all calls below): `hwnd` is our window, on its own thread.
    match msg {
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        // Only reaches us in move mode: otherwise the window is transparent.
        WM_LBUTTONDOWN if interactive => {
            unsafe { SetCapture(hwnd) };
            DRAGGING.set(true);
            push(Event::DragStart(cursor()));
        }
        WM_MOUSEMOVE if DRAGGING.get() => push(Event::DragMove(cursor())),
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            // Releasing the capture sends WM_CAPTURECHANGED, which then
            // finds the drag already over. Losing the capture otherwise
            // (Alt+Tab, a dialog) ends the drag where the pointer is.
            if DRAGGING.replace(false) {
                push(Event::DragEnd(cursor()));
                unsafe { ReleaseCapture() };
            }
        }
        WM_KEYDOWN if interactive => {
            if let Some(code) = key_code(wparam) {
                let shift = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
                push(Event::Key(Key { code, shift }));
            }
        }
        WM_DISPLAYCHANGE => displays_changed(hwnd, "WM_DISPLAYCHANGE"),
        WM_DPICHANGED => displays_changed(hwnd, "WM_DPICHANGED"),
        WM_SETTINGCHANGE if wparam == SPI_SETWORKAREA as WPARAM => {
            displays_changed(hwnd, "work area changed")
        }
        WM_CLOSE => push(Event::Command(Command::Quit)),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

/// Every monitor (SPEC §11.1.3), in physical pixels.
fn monitors() -> Vec<DisplayInfo> {
    unsafe extern "system" fn add(m: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the Vec passed to EnumDisplayMonitors below,
        // alive for the whole enumeration; `info` is sized for the EX form.
        unsafe {
            let list = &mut *(list as *mut Vec<DisplayInfo>);
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(m, (&raw mut info).cast::<MONITORINFO>()) == 0 {
                return 1;
            }
            let (mut dpi, mut _y) = (96, 96);
            if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y) != 0 {
                dpi = 96;
            }
            let device = &info.szDevice;
            let len = device.iter().position(|&c| c == 0).unwrap_or(device.len());
            let name = String::from_utf16_lossy(&device[..len]);
            let mi = info.monitorInfo;
            list.push(DisplayInfo {
                // `\\.\DISPLAY1` → `DISPLAY1`
                name: name.trim_start_matches(r"\\.\").to_string(),
                bounds: rect(mi.rcMonitor),
                work_area: rect(mi.rcWork),
                scale: f64::from(dpi) / 96.0,
                primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        1
    }
    let mut list: Vec<DisplayInfo> = Vec::new();
    // SAFETY: `add` only touches `list` during this call.
    unsafe { EnumDisplayMonitors(null_mut(), null(), Some(add), (&raw mut list) as LPARAM) };
    list
}

fn rect(r: RECT) -> PhysicalRect {
    PhysicalRect::new(
        r.left,
        r.top,
        (r.right - r.left).max(0) as u32,
        (r.bottom - r.top).max(0) as u32,
    )
}

/// The badge window and the DIB section its frames are copied into.
struct Window {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
    size: (u32, u32),
    placed: bool,
    presented: bool,
    shown: bool,
    /// The foreground window when move mode began.
    previous_focus: HWND,
}

impl Window {
    fn create() -> std::io::Result<Window> {
        let class = wide(CLASS);
        let title = wide("PAWPRINT");
        // SAFETY: the strings outlive the calls; the class lives for the
        // process.
        unsafe {
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                // Shown only in move mode: otherwise the pointer passes through.
                hCursor: LoadCursorW(null_mut(), IDC_SIZEALL),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                EX_STYLE,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Window {
                hwnd,
                dc: CreateCompatibleDC(null_mut()),
                bitmap: null_mut(),
                bits: null_mut(),
                size: (0, 0),
                placed: false,
                presented: false,
                shown: false,
                previous_focus: null_mut(),
            })
        }
    }

    /// A top-down 32-bit DIB section of this size, selected into `dc`.
    fn resize(&mut self, width: u32, height: u32) {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        // SAFETY: `info` describes the bitmap; the old bitmap is deleted
        // only after the new one replaces it in `dc`.
        unsafe {
            let bitmap = CreateDIBSection(self.dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() {
                log::error!(
                    "windows: CreateDIBSection {width}x{height}: {}",
                    std::io::Error::last_os_error()
                );
                return;
            }
            SelectObject(self.dc, bitmap);
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            self.bitmap = bitmap;
        }
        self.bits = bits.cast();
        self.size = (width, height);
    }

    /// Show the window once it has both a place and pixels.
    fn show(&mut self) {
        if self.placed && self.presented && !self.shown {
            // SAFETY: our window.
            unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
            self.shown = true;
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: handles we created and nothing else owns.
        unsafe {
            DestroyWindow(self.hwnd);
            DeleteDC(self.dc);
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
        }
    }
}

impl Surface for Window {
    fn displays(&self) -> Vec<DisplayInfo> {
        monitors()
    }

    fn present(&mut self, frame: &Frame) {
        if self.size != (frame.width, frame.height) {
            self.resize(frame.width, frame.height);
        }
        let len = frame.width as usize * frame.height as usize * 4;
        if self.bits.is_null() || self.size != (frame.width, frame.height) || frame.bgra.len() < len
        {
            return;
        }
        let size = SIZE {
            cx: frame.width as i32,
            cy: frame.height as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: the DIB holds exactly `len` bytes; a null destination
        // point keeps the window where `place` put it.
        let ok = unsafe {
            std::ptr::copy_nonoverlapping(frame.bgra.as_ptr(), self.bits, len);
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                null(),
                &size,
                self.dc,
                &POINT::default(),
                0,
                &blend,
                ULW_ALPHA,
            )
        };
        if ok == 0 {
            log::warn!(
                "windows: UpdateLayeredWindow: {}",
                std::io::Error::last_os_error()
            );
        }
        self.presented = true;
        self.show();
    }

    fn place(&mut self, r: PhysicalRect) {
        // SAFETY: our window. HWND_TOPMOST re-asserts the z-order each time.
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                r.x,
                r.y,
                r.width as i32,
                r.height as i32,
                SWP_NOACTIVATE,
            )
        };
        self.placed = true;
        self.show();
    }

    fn set_interactive(&mut self, on: bool) {
        INTERACTIVE.set(on);
        // SAFETY: our window; the previous foreground window may be gone,
        // which only makes SetForegroundWindow fail.
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as WINDOW_EX_STYLE;
            let ex = if on {
                ex & !WS_EX_TRANSPARENT
            } else {
                ex | WS_EX_TRANSPARENT
            };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex as isize);
            if on {
                // Arrow keys need the focus. Windows grants it only if the
                // foreground process allowed it (`pawprint move` does,
                // through `allow_foreground`); dragging works regardless.
                self.previous_focus = GetForegroundWindow();
                SetForegroundWindow(self.hwnd);
            } else {
                if DRAGGING.get() {
                    ReleaseCapture();
                }
                // Hand the keyboard back to whoever had it.
                if GetForegroundWindow() == self.hwnd && !self.previous_focus.is_null() {
                    SetForegroundWindow(self.previous_focus);
                }
                self.previous_focus = null_mut();
            }
        }
    }
}

/// Called by `pawprint move` in the terminal, which usually owns the
/// foreground: lets the badge take the keyboard for arrow-key nudging. The
/// permission lasts until the next user input.
pub fn allow_foreground() {
    // SAFETY: no preconditions; fails harmlessly without foreground rights.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

/// `pawprint preview` (SPEC §9). Hook for M4: run the same loop as [`run`]
/// without IPC, with the window made normal while `shell.is_preview()`:
/// focusable, closable, with a title, not topmost, not click-through; Esc
/// and closing send `Command::Quit`. The shell already centers the badge,
/// cycles every action, and draws it on a backdrop with a margin.
pub fn preview(shell: Shell) -> CmdResult {
    let _ = shell;
    Err((
        crate::exit::ERROR,
        "preview: not implemented on this platform yet (see ROADMAP.md M4)".into(),
    ))
}

pub fn doctor(report: &mut Report) {
    let aware = dpi_aware();
    report.section("Windows");
    report.row("backend", "windows (layered topmost tool window)");
    report.row(
        "dpi awareness",
        if aware {
            "per-monitor v2"
        } else {
            "unavailable (needs Windows 10 1703 or newer)"
        },
    );
    if !aware {
        report.problem("per-monitor DPI awareness is unavailable; the badge may be blurry or misplaced on scaled displays");
    }
    // SAFETY: no preconditions.
    let remote = unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0;
    report.row(
        "desktop",
        if remote {
            "remote (RDP session)"
        } else {
            "console"
        },
    );
    if let Ok(key) = crate::instance::session_key()
        && key.rsplit_once('-').is_some_and(|(_, sid)| sid == "0")
    {
        report.problem(
            "this is session 0 (services), which has no visible desktop; \
             run PAWPRINT from a logged-in desktop session",
        );
    }
    report.row(
        "capture",
        "never excluded from screen capture, so remote tools show the badge",
    );
    let displays = monitors();
    report.displays(&displays, select_display(&displays, "primary"));
    report.row("tray", "notification area (the tray icon arrives in M4)");
}
