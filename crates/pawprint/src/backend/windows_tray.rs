//! The Windows tray (SPEC §11.1.5): one notification-area icon on the badge
//! window, whose menu is `pawprint_core::menu`'s model. The badge's
//! `wndproc` passes its messages to [`message`], which records the chosen
//! command for the loop; the loop hands it to `Shell::menu`.
//!
//! What `wndproc` needs (the icon data and the menu model) lives in a
//! thread-local, so neither opening the menu nor adding the icon again
//! after Explorer restarts needs the shell.

use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};

use pawprint_core::badge::Command;
use pawprint_core::menu::{MenuItem, TrayState};
use pawprint_core::render::{Frame, tray_icon};
use pawprint_core::sprite::RgbaImage;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::windows::wide;
use crate::shell::Tray;

/// The tray icon's callback message.
pub const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
/// `NIN_SELECT | NINF_KEY`: Enter or Space on the focused icon.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

/// The icon as last shown.
struct Shown {
    data: NOTIFYICONDATAW,
    menu: Vec<MenuItem>,
    icon: RgbaImage,
    size: i32,
}

thread_local! {
    static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
    /// The registered `TaskbarCreated` message (Explorer restarted).
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    /// The foreground window when the menu last opened, before the menu
    /// took the foreground: where Move from the menu returns the keyboard.
    static BEFORE_MENU: Cell<HWND> = const { Cell::new(null_mut()) };
}

/// The foreground window when the tray menu last opened.
pub fn foreground_before_menu() -> HWND {
    BEFORE_MENU.get()
}

/// The tray, attached to the badge window `hwnd`.
pub struct NotifyIcon {
    hwnd: HWND,
}

impl NotifyIcon {
    pub fn new(hwnd: HWND) -> NotifyIcon {
        // SAFETY: a NUL-terminated name; our window. An elevated badge
        // would otherwise never hear from Explorer, which runs unelevated.
        unsafe {
            let created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            TASKBAR_CREATED.set(created);
            for msg in [created, WM_TRAY] {
                ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, null_mut());
            }
        }
        NotifyIcon { hwnd }
    }
}

impl Tray for NotifyIcon {
    fn show(&mut self, state: &TrayState) {
        let old = SHOWN.take();
        if !state.visible {
            if let Some(old) = old {
                log::info!("tray: off (tray = false)");
                remove_icon(&old.data);
            }
            return;
        }
        let size = icon_size();
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: make_icon(&state.icon, size),
            ..Default::default()
        };
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let tip: Vec<u16> = state
            .tooltip
            .encode_utf16()
            .take(data.szTip.len() - 1)
            .collect();
        data.szTip[..tip.len()].copy_from_slice(&tip);
        match &old {
            // SAFETY: `data` is a complete NOTIFYICONDATAW; the old icon is ours.
            Some(was) => unsafe {
                Shell_NotifyIconW(NIM_MODIFY, &data);
                DestroyIcon(was.data.hIcon);
            },
            None if add_icon(&data) => log::info!("tray: notification area icon added"),
            // Explorer isn't running yet; TaskbarCreated adds it.
            None => log::info!("tray: no notification area yet; waiting for Explorer"),
        }
        SHOWN.set(Some(Shown {
            data,
            menu: state.menu.clone(),
            icon: state.icon.clone(),
            size,
        }));
    }
}

impl Drop for NotifyIcon {
    fn drop(&mut self) {
        if let Some(shown) = SHOWN.take() {
            remove_icon(&shown.data);
        }
    }
}

/// Handle a tray message ([`is_tray_message`]) for the badge window: open
/// the menu and return the chosen command, or add the icon again after
/// Explorer restarts.
pub fn message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<Command> {
    if msg == WM_TRAY {
        // Left click, keyboard, and right click all open the menu at the
        // point the shell gives (NOTIFYICON_VERSION_4).
        if let NIN_SELECT | NIN_KEYSELECT | WM_CONTEXTMENU = (lparam & 0xffff) as u32 {
            let (x, y) = (i32::from(wparam as i16), i32::from((wparam >> 16) as i16));
            return tray_menu(hwnd, x, y);
        }
    } else if msg != 0 && msg == TASKBAR_CREATED.get() {
        log::info!("tray: Explorer restarted; adding the icon again");
        SHOWN.with_borrow(|s| s.as_ref().map(|s| add_icon(&s.data)));
    }
    None
}

/// Whether `msg` is one [`message`] handles.
pub fn is_tray_message(msg: u32) -> bool {
    msg == WM_TRAY || (msg != 0 && msg == TASKBAR_CREATED.get())
}

/// The scale or the displays changed: redraw the icon if the system's
/// small-icon size changed with them.
pub fn rescale() {
    let size = icon_size();
    SHOWN.with_borrow_mut(|s| {
        let Some(s) = s.as_mut().filter(|s| s.size != size) else {
            return;
        };
        log::debug!("tray: icon size {} -> {size}", s.size);
        let old = s.data.hIcon;
        s.data.hIcon = make_icon(&s.icon, size);
        s.size = size;
        // SAFETY: a complete NOTIFYICONDATAW; the old icon is ours.
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &s.data);
            DestroyIcon(old);
        }
    });
}

fn add_icon(data: &NOTIFYICONDATAW) -> bool {
    // SAFETY: `data` is a complete NOTIFYICONDATAW.
    unsafe { Shell_NotifyIconW(NIM_ADD, data) != 0 && Shell_NotifyIconW(NIM_SETVERSION, data) != 0 }
}

fn remove_icon(data: &NOTIFYICONDATAW) {
    // SAFETY: as above; the icon handle is ours.
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, data);
        DestroyIcon(data.hIcon);
    }
}

/// The small-icon size (16 px at 100%, 32 at 200%) at the DPI of the
/// primary display, where the taskbar's notification area is.
fn icon_size() -> i32 {
    // SAFETY: no preconditions; a failed DPI query keeps 96.
    unsafe {
        let primary = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let (mut dpi, mut _y) = (96, 96);
        if GetDpiForMonitor(primary, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y) != 0 {
            dpi = 96;
        }
        GetSystemMetricsForDpi(SM_CXSMICON, dpi).clamp(16, 256)
    }
}

/// The rest frame as a `size`-pixel icon (nearest neighbor, binary alpha).
fn make_icon(img: &RgbaImage, size: i32) -> HICON {
    // Binary alpha: premultiplied BGRA is also straight BGRA.
    let bgra = Frame::from_rgba(&tray_icon(img, size as u32, 0)).bgra;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    // The alpha channel decides; the mask must exist but is ignored.
    let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    // SAFETY: the DIB holds `bgra.len()` bytes; the icon copies both
    // bitmaps, which are deleted after.
    unsafe {
        let mut bits = null_mut();
        let color = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() {
            return null_mut();
        }
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len());
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// Open the tray menu at (x, y) and return the chosen command. The classic
/// popup-menu rule: the owner must be the foreground window, or the menu
/// does not close when the user clicks elsewhere, and a posted `WM_NULL`
/// after it lets the next click open it again.
fn tray_menu(hwnd: HWND, x: i32, y: i32) -> Option<Command> {
    fn build(items: &[MenuItem], commands: &mut Vec<Command>) -> HMENU {
        // SAFETY: AppendMenuW copies each label; a submenu belongs to its
        // parent and is destroyed with it.
        unsafe {
            let menu = CreatePopupMenu();
            for item in items {
                let label = wide(&item.label);
                if !item.children.is_empty() {
                    let sub = build(&item.children, commands);
                    AppendMenuW(menu, MF_POPUP, sub as usize, label.as_ptr());
                } else if let Some(command) = item.command {
                    commands.push(command);
                    let check = if item.checked == Some(true) {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                    AppendMenuW(menu, MF_STRING | check, commands.len(), label.as_ptr());
                }
            }
            menu
        }
    }
    let mut commands = Vec::new();
    // Built and released before the menu's modal loop, which may re-enter
    // `wndproc`.
    let menu = SHOWN.with_borrow(|s| s.as_ref().map(|s| build(&s.menu, &mut commands)))?;
    // SAFETY: our window and our menu.
    let id = unsafe {
        let before = GetForegroundWindow();
        BEFORE_MENU.set(if before == hwnd { null_mut() } else { before });
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            x,
            y,
            0,
            hwnd,
            null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        id
    };
    usize::try_from(id)
        .ok()
        .and_then(|id| commands.get(id.checked_sub(1)?).copied())
}
