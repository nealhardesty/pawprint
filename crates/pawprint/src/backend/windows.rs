//! Windows backend (SPEC §11.1): a layered, topmost, click-through tool
//! window, drawn with `UpdateLayeredWindow` from the shared premultiplied
//! BGRA frame, and a notification-area icon whose menu is the core's tray
//! menu (`windows_tray`).
//!
//! The window procedure only records what happened, as core events and tray
//! choices, in thread-local queues, and the loop hands them to the shell.
//! Win32 calls such as `SetWindowPos` send messages synchronously, so this
//! keeps the shell from being re-entered while it applies an effect.
//!
//! `pawprint preview` and `pawprint settings` use the same window and loop
//! with the `preview` flag: a normal titled, closable window painted with
//! `BitBlt`, whose input goes to `Shell::input`; no IPC, no tray.

use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};
use std::sync::mpsc;

use pawprint_core::badge::{Command, Event};
use pawprint_core::config::{Backend, Fullscreen};
use pawprint_core::geom::{PhysicalRect, Point};
use pawprint_core::movemode::{Key, KeyCode};
use pawprint_core::render::Frame;
use pawprint_core::surface::{DisplayInfo, Surface, fullscreen_covers};
use pawprint_core::ui::{Input, UiKey};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BeginPaint,
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, EndPaint, EnumDisplayMonitors, GetMonitorInfoW, HBITMAP, HDC, HMONITOR,
    InvalidateRect, MONITORINFO, MONITORINFOEXW, PAINTSTRUCT, SRCCOPY, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, INFINITE};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::Shell::{
    ABM_NEW, ABM_REMOVE, ABN_FULLSCREENAPP, APPBARDATA, SHAppBarMessage,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::BOOL;

use super::windows_tray::{self, NotifyIcon};
use crate::doctor::Report;
use crate::shell::{PREVIEW_BACKDROP, Shell};
use crate::{CmdResult, exit, ipc};

const CLASS: &str = "PawprintBadge";
const PREVIEW_CLASS: &str = "PawprintPreview";

/// Posted by the IPC thread to wake the loop.
const WM_WAKE: u32 = WM_APP;
/// The shell's appbar notifications (`ABN_FULLSCREENAPP`), SPEC §6.5.
const WM_APPBAR: u32 = WM_APP + 1;

/// The preview: a normal window with a title, a close box, and a taskbar
/// button (SPEC §9).
const PREVIEW_STYLE: WINDOW_STYLE = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;

/// `WS_EX_TRANSPARENT` is cleared in move mode, so the badge takes the
/// pointer. A click never activates it (`WS_EX_NOACTIVATE`).
const EX_STYLE: WINDOW_EX_STYLE =
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;

thread_local! {
    /// Events `wndproc` recorded for the loop.
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
    /// Tray menu choices `wndproc` recorded for the loop.
    static CHOICES: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    /// The normal window's input, for `Shell::input`.
    static INPUTS: RefCell<Vec<Input>> = const { RefCell::new(Vec::new()) };
    static INTERACTIVE: Cell<bool> = const { Cell::new(false) };
    static DRAGGING: Cell<bool> = const { Cell::new(false) };
    static PREVIEW: Cell<bool> = const { Cell::new(false) };
    /// The foreground window changed or a fullscreen app opened or closed:
    /// look again (SPEC §6.5).
    static RECHECK: Cell<bool> = const { Cell::new(false) };
    /// The preview's memory DC and its frame's size, which `WM_PAINT`
    /// copies to the window.
    static PAINT: Cell<(HDC, i32, i32)> = const { Cell::new((null_mut(), 0, 0)) };
}

fn push(event: Event) {
    EVENTS.with_borrow_mut(|events| events.push(event));
}

fn push_input(input: Input) {
    INPUTS.with_borrow_mut(|inputs| inputs.push(input));
}

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Per-monitor DPI awareness v2 (SPEC §11.1.2), so every coordinate is in
/// physical pixels. False on Windows older than 10 1703.
fn dpi_aware() -> bool {
    // SAFETY: no preconditions; fails harmlessly if already set.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 }
}

pub fn run(shell: Shell, listener: ipc::Listener) -> CmdResult {
    drive(shell, Some(listener))
}

/// `pawprint preview` and `pawprint settings` (SPEC §9): the same window
/// and loop with the window made normal (`Shell::is_window`), without IPC
/// or a tray; its input goes to `Shell::input`.
pub fn window(shell: Shell) -> CmdResult {
    drive(shell, None)
}

/// The loop for the badge and the preview (SPEC §11.1.1). `listener` is
/// `None` for a preview.
fn drive(mut shell: Shell, listener: Option<ipc::Listener>) -> CmdResult {
    if !dpi_aware() {
        log::warn!("per-monitor DPI awareness unavailable; positions may be scaled");
    }
    let configured = shell.configured_backend();
    if !matches!(configured, Backend::Auto | Backend::Windows) {
        log::warn!("backend = \"{configured}\" does not exist on Windows; using windows");
    }
    shell.set_backend("windows");
    let title = shell.is_window().then(|| shell.title());
    let mut window = Window::create(title).map_err(|e| {
        (
            exit::NO_SESSION,
            format!("cannot create the badge window: {e}. Run `pawprint doctor`."),
        )
    })?;

    let (tx, rx) = mpsc::channel::<ipc::Incoming>();
    // HWND is a pointer and not Send; the IPC thread only posts to it.
    let hwnd = window.hwnd as isize;
    let has_tray = listener.is_some();
    if let Some(listener) = listener {
        ipc::serve(listener, move |incoming| {
            // SAFETY: posting to a window that may be gone just fails.
            tx.send(incoming).is_ok() && unsafe { PostMessageW(hwnd as HWND, WM_WAKE, 0, 0) } != 0
        });
    }

    shell.start(&mut window);
    if has_tray {
        shell.set_tray(Box::new(NotifyIcon::new(window.hwnd)));
    }
    loop {
        pump();
        for event in EVENTS.take() {
            shell.handle(&mut window, event);
        }
        for input in INPUTS.take() {
            shell.input(&mut window, input);
        }
        for command in CHOICES.take() {
            shell.menu(&mut window, command);
        }
        if RECHECK.take() {
            window.update_cover();
        }
        while let Ok(incoming) = rx.try_recv() {
            shell.request(&mut window, incoming);
        }
        shell.tick(&mut window);
        if shell.done() {
            // The tray icon goes with the shell, while its window still exists.
            drop(shell);
            return Ok(());
        }
        // Applying effects can send messages that record new events.
        let pending = EVENTS.with_borrow(|e| !e.is_empty())
            || CHOICES.with_borrow(|c| !c.is_empty())
            || INPUTS.with_borrow(|i| !i.is_empty());
        let wait = if pending {
            0
        } else {
            shell.timeout().map_or(INFINITE, |d| {
                u32::try_from(d.as_micros().div_ceil(1000)).unwrap_or(INFINITE - 1)
            })
        };
        // SAFETY: no handles; wakes on any input or posted message, even
        // input that arrived before the last peek (MWMO_INPUTAVAILABLE).
        unsafe { MsgWaitForMultipleObjectsEx(0, null(), wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
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
        // WM_CHAR for the normal window's text fields.
        unsafe { TranslateMessage(&msg) };
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

/// The normal window's editing keys, and Ctrl+letter shortcuts.
fn window_key(vk: WPARAM) -> Option<Input> {
    let vk = u16::try_from(vk).ok()?;
    // SAFETY: no preconditions.
    let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
    if ctrl && (0x41..=0x5a).contains(&vk) {
        return Some(Input::Shortcut(char::from(vk as u8).to_ascii_lowercase()));
    }
    Some(Input::Key(match vk {
        VK_LEFT => UiKey::Left,
        VK_RIGHT => UiKey::Right,
        VK_UP => UiKey::Up,
        VK_DOWN => UiKey::Down,
        VK_HOME => UiKey::Home,
        VK_END => UiKey::End,
        VK_PRIOR => UiKey::PageUp,
        VK_NEXT => UiKey::PageDown,
        VK_BACK => UiKey::Backspace,
        VK_DELETE => UiKey::Delete,
        VK_RETURN => UiKey::Enter,
        VK_ESCAPE => UiKey::Escape,
        VK_TAB => UiKey::Tab,
        _ => return None,
    }))
}

/// A client-area point of the normal window in the frame's coordinates:
/// `WM_PAINT` centers a frame narrower than the window.
fn frame_point(hwnd: HWND, lparam: LPARAM) -> Point {
    let (x, y) = (
        lparam as u32 as u16 as i16,
        (lparam as u32 >> 16) as u16 as i16,
    );
    let mut client = RECT::default();
    // SAFETY: our window.
    unsafe { GetClientRect(hwnd, &mut client) };
    let (_, w, _) = PAINT.get();
    Point::new(i32::from(x) - (client.right - w) / 2, i32::from(y))
}

/// A display, resolution, scale, or work area changed: stay on top (SPEC
/// §11.1.3) and report the new set; the shell drops repeats.
fn displays_changed(hwnd: HWND, why: &str) {
    log::debug!("windows: {why}");
    if !PREVIEW.get() {
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
    }
    windows_tray::rescale();
    RECHECK.set(true);
    push(Event::DisplaysChanged(monitors()));
}

/// `EVENT_SYSTEM_FOREGROUND`, delivered through our message queue
/// (`WINEVENT_OUTOFCONTEXT`): another window came to the front.
unsafe extern "system" fn foreground_changed(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    RECHECK.set(true);
}

/// The foreground window's rectangle if it may be fullscreen: not the
/// desktop, not ours, not minimized, and without a caption, so a window
/// maximized under an auto-hidden taskbar doesn't count. Whether it covers
/// the badge's display is the core's call (`fullscreen_covers`).
fn foreground_fullscreen() -> Option<PhysicalRect> {
    // SAFETY: read-only queries on a window that may vanish, which only
    // makes them fail.
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || fg == GetShellWindow() || fg == GetDesktopWindow() || IsIconic(fg) != 0 {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(fg, &mut pid);
        if pid == GetCurrentProcessId() {
            return None;
        }
        let mut class = [0u16; 16];
        let n = GetClassNameW(fg, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if class == "Progman" || class == "WorkerW" {
            return None;
        }
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        if style & WS_CAPTION == WS_CAPTION {
            return None;
        }
        let mut r = RECT::default();
        (GetWindowRect(fg, &mut r) != 0).then(|| rect(r))
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let interactive = INTERACTIVE.get();
    let preview = PREVIEW.get();
    // SAFETY (all calls below): `hwnd` is our window, on its own thread.
    match msg {
        WM_MOUSEACTIVATE if !preview => return MA_NOACTIVATE as LRESULT,
        WM_KEYDOWN if preview => {
            if let Some(input) = window_key(wparam) {
                push_input(input);
            }
        }
        WM_CHAR if preview => {
            if let Some(c) = char::from_u32(wparam as u32).filter(|c| !c.is_control()) {
                push_input(Input::Text(c));
            }
        }
        WM_MOUSEMOVE if preview => push_input(Input::Move(frame_point(hwnd, lparam))),
        WM_LBUTTONDOWN if preview => {
            unsafe { SetCapture(hwnd) };
            push_input(Input::Press(frame_point(hwnd, lparam)));
        }
        WM_LBUTTONUP if preview => {
            unsafe { ReleaseCapture() };
            push_input(Input::Release(frame_point(hwnd, lparam)));
        }
        WM_MOUSEWHEEL if preview => {
            let notches = f64::from((wparam >> 16) as u16 as i16) / 120.0;
            push_input(Input::Scroll(-3.0 * notches));
        }
        // Windows keeps a captioned window at least `SM_CXMIN` wide, so a
        // small preview is centered on the class's backdrop brush.
        WM_PAINT if preview => unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let (src, w, h) = PAINT.get();
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client);
            BitBlt(dc, (client.right - w) / 2, 0, w, h, src, 0, 0, SRCCOPY);
            EndPaint(hwnd, &ps);
        },
        WM_APPBAR if wparam == ABN_FULLSCREENAPP as WPARAM => RECHECK.set(true),
        m if windows_tray::is_tray_message(m) => {
            if let Some(command) = windows_tray::message(hwnd, m, wparam, lparam) {
                CHOICES.with_borrow_mut(|c| c.push(command));
            }
        }
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
    preview: bool,
    /// Where the badge was last placed.
    rect: Option<PhysicalRect>,
    /// Watching for fullscreen windows: an appbar (for `ABN_FULLSCREENAPP`)
    /// and a foreground hook. `None` with `fullscreen = "show"`.
    watch: Option<HWINEVENTHOOK>,
    /// A fullscreen window covers the badge's display, so it is hidden.
    covered: bool,
}

impl Window {
    /// The badge, or with a title a normal window.
    fn create(title: Option<&str>) -> std::io::Result<Window> {
        let preview = title.is_some();
        PREVIEW.set(preview);
        let (class, title, cursor, ex, style) = if let Some(title) = title {
            (PREVIEW_CLASS, title, IDC_ARROW, 0, PREVIEW_STYLE)
        } else {
            // The move cursor shows only in move mode: otherwise the
            // pointer passes through.
            (CLASS, "PAWPRINT", IDC_SIZEALL, EX_STYLE, WS_POPUP)
        };
        let (class, title) = (wide(class), wide(title));
        // SAFETY: the strings outlive the calls; the class lives for the
        // process.
        unsafe {
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), cursor),
                hbrBackground: if preview {
                    let [r, g, b] = PREVIEW_BACKDROP.map(u32::from);
                    CreateSolidBrush(r | g << 8 | b << 16)
                } else {
                    null_mut()
                },
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                ex,
                class.as_ptr(),
                title.as_ptr(),
                style,
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
            let dc = CreateCompatibleDC(null_mut());
            Ok(Window {
                hwnd,
                dc,
                bitmap: null_mut(),
                bits: null_mut(),
                size: (0, 0),
                placed: false,
                presented: false,
                shown: false,
                previous_focus: null_mut(),
                preview,
                rect: None,
                watch: None,
                covered: false,
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

    fn appbar(&self) -> APPBARDATA {
        APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            uCallbackMessage: WM_APPBAR,
            ..Default::default()
        }
    }

    /// Hide the badge while a fullscreen window covers its display, show it
    /// again after (SPEC §6.5).
    fn update_cover(&mut self) {
        let covered = self.watch.is_some()
            && self.rect.is_some_and(|badge| {
                foreground_fullscreen().is_some_and(|w| fullscreen_covers(w, badge, &monitors()))
            });
        if covered == self.covered {
            return;
        }
        log::info!(
            "windows: {}",
            if covered {
                "a fullscreen window covers the badge's display: hiding"
            } else {
                "no fullscreen window over the badge: showing"
            }
        );
        self.covered = covered;
        if self.shown {
            // SAFETY: our window.
            unsafe { ShowWindow(self.hwnd, if covered { SW_HIDE } else { SW_SHOWNOACTIVATE }) };
        }
    }

    /// Show the window once it has both a place and pixels.
    fn show(&mut self) {
        if self.placed && self.presented && !self.shown && !self.covered {
            let how = if self.preview {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            };
            // SAFETY: our window.
            unsafe { ShowWindow(self.hwnd, how) };
            self.shown = true;
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: handles we created and nothing else owns.
        unsafe {
            if let Some(hook) = self.watch.take() {
                SHAppBarMessage(ABM_REMOVE, &mut self.appbar());
                if !hook.is_null() {
                    UnhookWinEvent(hook);
                }
            }
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
        if self.preview {
            // A normal window: `WM_PAINT` copies the DIB to it.
            // SAFETY: the DIB holds exactly `len` bytes; our window.
            unsafe {
                std::ptr::copy_nonoverlapping(frame.bgra.as_ptr(), self.bits, len);
                InvalidateRect(self.hwnd, null(), 0);
            }
            PAINT.set((self.dc, frame.width as i32, frame.height as i32));
            self.presented = true;
            return self.show();
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
        if self.preview {
            // `r` is the client area; the frame goes around it.
            let mut outer = RECT {
                left: r.x,
                top: r.y,
                right: r.x + r.width as i32,
                bottom: r.y + r.height as i32,
            };
            // SAFETY: our window.
            unsafe {
                let dpi = GetDpiForWindow(self.hwnd);
                AdjustWindowRectExForDpi(&mut outer, PREVIEW_STYLE, 0, 0, dpi);
                // Stay centered when Windows widens it to its minimum.
                let grow = GetSystemMetricsForDpi(SM_CXMIN, dpi) - (outer.right - outer.left);
                if grow > 0 {
                    outer.left -= grow / 2;
                    outer.right += grow - grow / 2;
                }
                SetWindowPos(
                    self.hwnd,
                    null_mut(),
                    outer.left,
                    outer.top,
                    outer.right - outer.left,
                    outer.bottom - outer.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            self.placed = true;
            return self.show();
        }
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
        self.rect = Some(r);
        self.placed = true;
        self.update_cover();
        self.show();
    }

    fn set_interactive(&mut self, on: bool) {
        if self.preview {
            // A normal window: never click-through, never in move mode.
            return;
        }
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
                // foreground process allowed it (`pawprint move` does, as
                // `ipc::windows` connects); dragging works regardless. Move
                // from the tray menu finds the badge in the foreground
                // already: the keyboard goes back to whoever had it before.
                let foreground = GetForegroundWindow();
                self.previous_focus = if foreground == self.hwnd {
                    windows_tray::foreground_before_menu()
                } else {
                    foreground
                };
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

    /// `Hide`: register as an appbar, so the shell sends
    /// `ABN_FULLSCREENAPP` when a fullscreen app opens or closes (it
    /// reserves no screen space without `ABM_SETPOS`), and hook foreground
    /// changes, which catch switching between a fullscreen window and
    /// others. Both arrive as messages: nothing polls.
    fn set_fullscreen(&mut self, policy: Fullscreen) {
        if self.preview {
            return;
        }
        let mut data = self.appbar();
        match (policy, self.watch) {
            (Fullscreen::Hide, None) => {
                // SAFETY: `data` names our window; the hook calls back on
                // this thread from our message loop.
                unsafe {
                    SHAppBarMessage(ABM_NEW, &mut data);
                    let hook = SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        null_mut(),
                        Some(foreground_changed),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    );
                    self.watch = Some(hook);
                }
            }
            (Fullscreen::Show, Some(hook)) => {
                // SAFETY: the appbar and hook we registered above.
                unsafe {
                    SHAppBarMessage(ABM_REMOVE, &mut data);
                    if !hook.is_null() {
                        UnhookWinEvent(hook);
                    }
                }
                self.watch = None;
            }
            _ => return,
        }
        self.update_cover();
    }
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
    report.displays(&displays);
    report.row("tray", "notification area icon (Shell_NotifyIconW)");
}
