//! Launch at graphical login (SPEC §9, §10.6): one interface, a small
//! implementation per platform. Every entry runs `<badge exe> run`, the same
//! command the detached start uses (`instance::badge_exe`).
//!
//! Linux: an XDG autostart entry. Windows (HKCU `Run`) and macOS
//! (LaunchAgent) are hooks for their backends to fill in (SPEC §11.1.5,
//! §11.2).

use std::path::PathBuf;

use crate::{CmdResult, exit, instance};

/// Whether this user's login starts the badge.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))] // until Windows and macOS land
pub enum State {
    /// On: where the entry lives and the command it runs.
    On {
        entry: PathBuf,
        command: String,
    },
    Off,
}

/// The command line an autostart entry runs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn command_line() -> Result<(PathBuf, String), String> {
    let exe = instance::badge_exe().map_err(|e| format!("cannot find this executable: {e}"))?;
    Ok((exe, "run".to_string()))
}

/// `pawprint autostart on|off|status`.
pub fn cli(action: crate::Toggle) -> CmdResult {
    let fail = |e: String| (exit::ERROR, e);
    match action {
        crate::Toggle::On => {
            let state = platform::enable().map_err(fail)?;
            println!("Autostart is on: {}", describe(&state));
        }
        crate::Toggle::Off => {
            let was = platform::status().map_err(fail)?;
            platform::disable().map_err(fail)?;
            println!(
                "{}",
                if was == State::Off {
                    "Autostart was already off."
                } else {
                    "Autostart is off. PAWPRINT keeps running until `pawprint quit` or logout."
                }
            );
        }
        crate::Toggle::Status => {
            let state = platform::status().map_err(fail)?;
            println!(
                "{}",
                match state {
                    State::Off => "off (`pawprint autostart on` starts the badge at login)".into(),
                    on => format!("on: {}", describe(&on)),
                }
            );
        }
    }
    Ok(())
}

/// For `doctor`.
pub fn status() -> Result<State, String> {
    platform::status()
}

pub fn describe(state: &State) -> String {
    match state {
        State::On { entry, command } => format!("{command} at login ({})", entry.display()),
        State::Off => "off".into(),
    }
}

#[cfg(target_os = "linux")]
use xdg as platform;

/// Linux: `$XDG_CONFIG_HOME/autostart/pawprint.desktop` (the freedesktop
/// autostart spec), honored by GNOME, KDE Plasma, Xfce, and most others.
#[cfg(target_os = "linux")]
mod xdg {
    use std::path::{Path, PathBuf};

    use super::State;

    fn entry_path() -> Result<PathBuf, String> {
        let base = directories::BaseDirs::new().ok_or("cannot find the home directory")?;
        Ok(base.config_dir().join("autostart").join("pawprint.desktop"))
    }

    /// Quote an `Exec` argument as the Desktop Entry spec asks: in double
    /// quotes, with `"`, `` ` ``, `$`, and `\` escaped, when it has any
    /// reserved character.
    pub fn quote(arg: &str) -> String {
        let reserved = |c: char| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c);
        if !arg.contains(reserved) {
            return arg.to_string();
        }
        let mut out = String::from("\"");
        for c in arg.chars() {
            if "\"`$\\".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        // The desktop file's own string escaping doubles backslashes.
        out.replace('\\', "\\\\")
    }

    pub fn entry(exe: &Path, arg: &str) -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=PAWPRINT\n\
             Comment=Pixel-art hostname badge in the corner of the screen\n\
             Exec={} {arg}\n\
             Terminal=false\n\
             NoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n",
            quote(&exe.to_string_lossy())
        )
    }

    fn exec_line(text: &str) -> Option<String> {
        text.lines()
            .find_map(|l| l.strip_prefix("Exec="))
            .map(str::to_string)
    }

    pub fn status() -> Result<State, String> {
        status_at(&entry_path()?)
    }

    pub fn status_at(path: &Path) -> Result<State, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(State::On {
                entry: path.to_path_buf(),
                command: exec_line(&text).unwrap_or_else(|| "(no Exec line)".into()),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::Off),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    pub fn enable() -> Result<State, String> {
        let (exe, arg) = super::command_line()?;
        enable_at(&entry_path()?, &exe, &arg)
    }

    pub fn enable_at(path: &Path, exe: &Path, arg: &str) -> Result<State, String> {
        let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        std::fs::write(path, entry(exe, arg)).map_err(fail)?;
        status_at(path)
    }

    pub fn disable() -> Result<(), String> {
        disable_at(&entry_path()?)
    }

    pub fn disable_at(path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("cannot remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        }
    }
}

/// Hooks for the Windows and macOS backends. Windows: the
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `pawprint` =
/// `"<dir>\pawprintw.exe" run` (`command_line` already picks pawprintw.exe).
/// macOS: `~/Library/LaunchAgents/io.github.nealhardesty.pawprint.plist`
/// (SPEC §11.2 lists its keys).
#[cfg(not(target_os = "linux"))]
mod platform {
    use super::State;

    const NOT_YET: &str = "autostart: not implemented on this platform yet (see ROADMAP.md M4)";

    pub fn status() -> Result<State, String> {
        Err(NOT_YET.into())
    }

    pub fn enable() -> Result<State, String> {
        Err(NOT_YET.into())
    }

    pub fn disable() -> Result<(), String> {
        Err(NOT_YET.into())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::xdg::*;
    use super::*;
    use std::path::Path;

    #[test]
    fn exec_quoting() {
        assert_eq!(
            quote("/home/a/.local/bin/pawprint"),
            "/home/a/.local/bin/pawprint"
        );
        assert_eq!(quote("/opt/my apps/pawprint"), "\"/opt/my apps/pawprint\"");
        assert_eq!(quote("/a$b"), "\"/a\\\\$b\"");
    }

    #[test]
    fn on_status_off() {
        let dir = std::env::temp_dir().join(format!("pawprint-autostart-{}", std::process::id()));
        let path = dir.join("autostart").join("pawprint.desktop");
        assert_eq!(status_at(&path), Ok(State::Off));
        let on = enable_at(&path, Path::new("/usr/local/bin/pawprint"), "run").unwrap();
        assert_eq!(
            on,
            State::On {
                entry: path.clone(),
                command: "/usr/local/bin/pawprint run".into()
            }
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("[Desktop Entry]\nType=Application\n"),
            "{text}"
        );
        disable_at(&path).unwrap();
        disable_at(&path).unwrap(); // already off is fine
        assert_eq!(status_at(&path), Ok(State::Off));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
