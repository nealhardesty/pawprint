//! The Linux tray (SPEC §11.4, §11.5.1): a StatusNotifierItem through `ksni`
//! on the session bus, for X11, Wayland, and GNOME with the AppIndicator
//! extension alike. It draws `pawprint_core::menu`'s model and posts each
//! choice into the badge's loop as an [`Inbox::Menu`] command, so the tray
//! and IPC share one command path (`Shell::menu`).
//!
//! Without a StatusNotifierWatcher the item waits quietly for one to appear
//! (ksni's `assume_sni_available`); `pawprint doctor` reports the missing
//! tray. Without a session bus there is no tray at all.

use calloop::channel::Sender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use pawprint_core::menu::{MenuItem, TrayState};
use pawprint_core::sprite::RgbaImage;

use super::linux::Inbox;
use crate::shell::Tray;

/// The bus name of the StatusNotifierWatcher every SNI host provides.
pub const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// The tray as the shell sees it. The ksni service starts on the first
/// visible state and stops when `tray = false`.
pub struct Sni {
    tx: Sender<Inbox>,
    handle: Option<Handle<Item>>,
    /// The service could not start (no session bus); don't retry.
    failed: bool,
}

impl Sni {
    pub fn new(tx: Sender<Inbox>) -> Sni {
        Sni {
            tx,
            handle: None,
            failed: false,
        }
    }
}

impl Tray for Sni {
    fn show(&mut self, state: &TrayState) {
        if !state.visible {
            if let Some(h) = self.handle.take() {
                log::info!("tray: off (tray = false)");
                h.shutdown().wait();
            }
            return;
        }
        if let Some(h) = &self.handle {
            let state = state.clone();
            h.update(move |item| item.state = state);
            return;
        }
        if self.failed {
            return;
        }
        let item = Item {
            state: state.clone(),
            tx: self.tx.clone(),
        };
        match item.assume_sni_available(true).spawn() {
            Ok(h) => {
                log::info!("tray: StatusNotifierItem registered");
                self.handle = Some(h);
            }
            Err(e) => {
                log::info!("tray: none ({e}); `pawprint doctor` explains");
                self.failed = true;
            }
        }
    }
}

/// The StatusNotifierItem itself, owned by ksni's thread.
pub struct Item {
    state: TrayState,
    tx: Sender<Inbox>,
}

impl ksni::Tray for Item {
    /// A left click opens the menu, as on Windows and macOS.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "pawprint".into()
    }

    fn title(&self) -> String {
        "PAWPRINT".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icons(&self.state.icon)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.state.tooltip.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        to_ksni(&self.state.menu)
    }

    /// Keep waiting for a watcher (a desktop that starts its panel later,
    /// or restarts it).
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        log::info!("tray: no StatusNotifierWatcher ({reason:?}); waiting for one");
        true
    }
}

/// The shared menu model as ksni items: submenus for children, check items
/// where the model has a check, plain items otherwise.
pub fn to_ksni(items: &[MenuItem]) -> Vec<ksni::MenuItem<Item>> {
    items
        .iter()
        .map(|i| {
            let label = i.label.clone();
            if !i.children.is_empty() {
                return SubMenu {
                    label,
                    submenu: to_ksni(&i.children),
                    ..Default::default()
                }
                .into();
            }
            let command = i.command;
            let activate = Box::new(move |item: &mut Item| {
                if let Some(c) = command {
                    let _ = item.tx.send(Inbox::Menu(c));
                }
            });
            match i.checked {
                Some(checked) => CheckmarkItem {
                    label,
                    checked,
                    activate,
                    ..Default::default()
                }
                .into(),
                None => StandardItem {
                    label,
                    activate,
                    ..Default::default()
                }
                .into(),
            }
        })
        .collect()
}

/// The icon at 32 and 64 pixels (nearest neighbor), ARGB32 in network byte
/// order as the SNI spec asks; the host picks and scales.
pub fn icons(img: &RgbaImage) -> Vec<ksni::Icon> {
    [1, 2]
        .into_iter()
        .map(|f| {
            let scaled = pawprint_core::render::scale(img, f);
            ksni::Icon {
                width: scaled.width as i32,
                height: scaled.height as i32,
                data: scaled
                    .data
                    .chunks(4)
                    .flat_map(|p| [p[3], p[0], p[1], p[2]])
                    .collect(),
            }
        })
        .collect()
}

/// Is a StatusNotifierWatcher on the session bus? (For `doctor`.)
pub fn watcher_present() -> zbus::Result<bool> {
    let conn = zbus::blocking::Connection::session()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    Ok(dbus.name_has_owner(WATCHER.try_into()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawprint_core::badge::Command;
    use pawprint_core::config::Animation;
    use pawprint_core::menu::menu;

    #[test]
    fn the_shared_menu_maps_onto_ksni_and_posts_commands() {
        let (tx, rx) = calloop::channel::channel::<Inbox>();
        let mut item = Item {
            state: TrayState {
                visible: true,
                icon: RgbaImage::new(32, 32),
                tooltip: "PAWPRINT: OWL (owl)".into(),
                menu: menu(64, Animation::Idle, false),
            },
            tx,
        };
        let items = to_ksni(&item.state.menu);
        let labels: Vec<String> = items
            .iter()
            .map(|i| match i {
                ksni::MenuItem::Standard(s) => s.label.clone(),
                ksni::MenuItem::Checkmark(c) => {
                    format!("[{}] {}", if c.checked { 'x' } else { ' ' }, c.label)
                }
                ksni::MenuItem::SubMenu(s) => format!("{} >", s.label),
                _ => "?".into(),
            })
            .collect();
        assert_eq!(
            labels,
            [
                "[ ] Move",
                "Size >",
                "Animation >",
                "Reset position",
                "Reload config",
                "Quit"
            ]
        );
        let ksni::MenuItem::SubMenu(size) = &items[1] else {
            panic!("Size is a submenu")
        };
        let sizes: Vec<(String, bool)> = size
            .submenu
            .iter()
            .map(|i| match i {
                ksni::MenuItem::Checkmark(c) => (c.label.clone(), c.checked),
                _ => panic!("check items"),
            })
            .collect();
        assert_eq!(
            sizes,
            [
                ("32".into(), false),
                ("64".into(), true),
                ("128".into(), false)
            ]
        );

        // Choosing "128" posts the same command the core's menu model holds.
        let ksni::MenuItem::Checkmark(big) = &size.submenu[2] else {
            unreachable!()
        };
        (big.activate)(&mut item);
        let ksni::MenuItem::Standard(quit) = &items[5] else {
            unreachable!()
        };
        (quit.activate)(&mut item);
        drop(item);
        let mut got = Vec::new();
        let mut events = calloop::EventLoop::<Vec<Command>>::try_new().unwrap();
        events
            .handle()
            .insert_source(rx, |e, _, got: &mut Vec<Command>| {
                if let calloop::channel::Event::Msg(Inbox::Menu(c)) = e {
                    got.push(c);
                }
            })
            .unwrap();
        events
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut got)
            .unwrap();
        assert_eq!(got, [Command::SetSize(128), Command::Quit]);
    }

    #[test]
    fn icons_are_argb_network_order_at_two_sizes() {
        let mut img = RgbaImage::new(32, 32);
        img.put(0, 0, [10, 20, 30, 255]);
        let icons = icons(&img);
        assert_eq!(
            icons
                .iter()
                .map(|i| (i.width, i.height))
                .collect::<Vec<_>>(),
            [(32, 32), (64, 64)]
        );
        assert_eq!(&icons[0].data[..4], &[255, 10, 20, 30]);
        assert_eq!(&icons[1].data[4..8], &[255, 10, 20, 30], "2x nearest");
        assert_eq!(&icons[1].data[8..12], &[0, 0, 0, 0]);
    }
}
