//! The tray / menu-bar menu, described once for every platform
//! (SPEC §11.1.5). Backends only draw it and report the chosen command.

use pawprint_sprite::RgbaImage;

use crate::badge::Command;
use crate::config::Animation;
use crate::update::{Notice, Version};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    /// What choosing the item does; `None` for a submenu.
    pub command: Option<Command>,
    /// `Some` for items shown with a check mark (radio-style groups).
    pub checked: Option<bool>,
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    fn action(label: &str, command: Command) -> MenuItem {
        MenuItem {
            label: label.into(),
            command: Some(command),
            checked: None,
            children: vec![],
        }
    }

    fn check(label: &str, command: Command, on: bool) -> MenuItem {
        MenuItem {
            checked: Some(on),
            ..MenuItem::action(label, command)
        }
    }

    fn submenu(label: &str, children: Vec<MenuItem>) -> MenuItem {
        MenuItem {
            label: label.into(),
            command: None,
            checked: None,
            children,
        }
    }
}

/// Everything a backend needs to show the tray icon and its menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayState {
    pub visible: bool,
    /// The rest frame in its crate, without the label, at 1× (32 × 32,
    /// straight RGBA, binary alpha). Backends scale it to the icon size.
    pub icon: RgbaImage,
    pub tooltip: String,
    pub menu: Vec<MenuItem>,
}

/// Sizes offered in the menu.
pub const SIZE_PRESETS: [u32; 3] = [32, 64, 128];

/// The update item (SPEC §15.8): it checks, until a check finds a newer
/// release, and then installs that. While a job runs it says so, and
/// choosing it again does nothing.
fn update_item(notice: Option<Notice>, available: Option<Version>) -> MenuItem {
    match (notice, available) {
        (Some(Notice::Checking), _) => {
            MenuItem::action("Checking for updates…", Command::CheckUpdate)
        }
        (Some(Notice::Updating), _) => MenuItem::action("Updating…", Command::InstallUpdate),
        (_, Some(v)) => MenuItem::action(&format!("Install update {v}"), Command::InstallUpdate),
        _ => MenuItem::action("Check for updates", Command::CheckUpdate),
    }
}

/// Move, Size ▸ 32/64/128, Animation ▸ Idle/Static, Settings…, Reset
/// position, Reload config, Check for updates (or Install update 1.3.0),
/// Quit.
pub fn menu(
    size: u32,
    animation: Animation,
    moving: bool,
    notice: Option<Notice>,
    available: Option<Version>,
) -> Vec<MenuItem> {
    let sizes = SIZE_PRESETS
        .iter()
        .map(|&n| MenuItem::check(&n.to_string(), Command::SetSize(n), size == n))
        .collect();
    vec![
        MenuItem::check("Move", Command::Move, moving),
        MenuItem::submenu("Size", sizes),
        MenuItem::submenu(
            "Animation",
            vec![
                MenuItem::check(
                    "Idle",
                    Command::SetAnimation(Animation::Idle),
                    animation == Animation::Idle,
                ),
                MenuItem::check(
                    "Static",
                    Command::SetAnimation(Animation::Static),
                    animation == Animation::Static,
                ),
            ],
        ),
        MenuItem::action("Settings…", Command::Settings),
        MenuItem::action("Reset position", Command::ResetPosition),
        MenuItem::action("Reload config", Command::Reload),
        update_item(notice, available),
        MenuItem::action("Quit", Command::Quit),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_shape_and_checks() {
        let m = menu(64, Animation::Static, false, None, None);
        let labels: Vec<&str> = m.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Move",
                "Size",
                "Animation",
                "Settings…",
                "Reset position",
                "Reload config",
                "Check for updates",
                "Quit"
            ]
        );
        let size_checks: Vec<Option<bool>> = m[1].children.iter().map(|i| i.checked).collect();
        assert_eq!(size_checks, [Some(false), Some(true), Some(false)]);
        assert_eq!(m[1].children[2].command, Some(Command::SetSize(128)));
        assert_eq!(m[2].children[1].checked, Some(true));
        assert_eq!(m[3].command, Some(Command::Settings));
        assert_eq!(m[6].command, Some(Command::CheckUpdate));
        assert_eq!(m[7].command, Some(Command::Quit));
        assert_eq!(
            menu(96, Animation::Idle, true, None, None)[1]
                .children
                .iter()
                .filter(|i| i.checked == Some(true))
                .count(),
            0
        );
        assert_eq!(
            menu(96, Animation::Idle, true, None, None)[0].checked,
            Some(true)
        );
    }

    #[test]
    fn the_update_item_follows_the_update() {
        let v: Version = "1.3.0".parse().unwrap();
        let item = |notice, available| {
            let m = menu(64, Animation::Idle, false, notice, available);
            (m[6].label.clone(), m[6].command)
        };
        assert_eq!(
            item(Some(Notice::Checking), None),
            ("Checking for updates…".into(), Some(Command::CheckUpdate))
        );
        // A result is on the crate for a few seconds; the menu already
        // offers the next step.
        for notice in [Some(Notice::Available(v)), None] {
            assert_eq!(
                item(notice, Some(v)),
                ("Install update 1.3.0".into(), Some(Command::InstallUpdate))
            );
        }
        assert_eq!(
            item(Some(Notice::Updating), Some(v)),
            ("Updating…".into(), Some(Command::InstallUpdate))
        );
        assert_eq!(
            item(Some(Notice::UpToDate), None),
            ("Check for updates".into(), Some(Command::CheckUpdate))
        );
    }
}
