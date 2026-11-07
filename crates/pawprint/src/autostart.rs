//! Launch at graphical login (SPEC §9, §10.6): one interface, a small
//! implementation per platform. Every entry runs `<badge exe> run`, the same
//! command the detached start uses (`instance::badge_exe`).
//!
//! Linux: an XDG autostart entry. Windows: the HKCU `Run` value (SPEC
//! §11.1.5). macOS: a LaunchAgent (SPEC §11.2).

use std::path::PathBuf;

use crate::{CmdResult, exit, instance};

/// Whether this user's login starts the badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// On: where the entry lives and the command it runs.
    On {
        entry: PathBuf,
        command: String,
    },
    Off,
}

/// The command line an autostart entry runs.
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
                } else if cfg!(target_os = "macos") {
                    "Autostart is off. A badge launchd started at login has stopped; `pawprint` \
                     starts one now."
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
    /// reserved character; and `%` doubled, since `%f` and friends are
    /// field codes.
    pub fn quote(arg: &str) -> String {
        let arg = &arg.replace('%', "%%");
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

    /// An entry with `Hidden=true` is the desktop's way of turning it off
    /// (the autostart spec), so it counts as off.
    pub fn status_at(path: &Path) -> Result<State, String> {
        match std::fs::read_to_string(path) {
            Ok(text) if text.lines().any(|l| l.trim() == "Hidden=true") => Ok(State::Off),
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

#[cfg(windows)]
use run_key as platform;

/// Windows: the `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value
/// `pawprint` = `"<dir>\pawprintw.exe" run`. HKCU Run entries run at every
/// interactive logon, RDP logons included (F10). Settings → Apps → Startup
/// (and Task Manager) can switch an entry off without removing it, in
/// `Explorer\StartupApproved\Run`: `status` says so, and `on` and `off`
/// clear that switch.
#[cfg(windows)]
mod run_key {
    use std::path::PathBuf;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW,
        RegGetValueW, RegSetKeyValueW,
    };

    use super::State;

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "pawprint";
    /// The Startup apps switch: a binary value whose first byte is odd
    /// (0x03) when the user turned the entry off, even (0x02) when on.
    const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn entry() -> PathBuf {
        PathBuf::from(format!(r"HKCU\{KEY}\{VALUE}"))
    }

    fn fail(what: &str, code: WIN32_ERROR) -> String {
        let e = std::io::Error::from_raw_os_error(code as i32);
        format!("cannot {what} {}: {e}", entry().display())
    }

    pub fn status() -> Result<State, String> {
        let (key, value) = (wide(KEY), wide(VALUE));
        let mut buf = vec![0u16; 1024];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `bytes` bytes; RRF_RT_REG_SZ NUL-terminates.
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
        match code {
            ERROR_SUCCESS => {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let mut command = String::from_utf16_lossy(&buf[..len]);
                if switched_off() {
                    command.push_str(
                        " (disabled in Startup apps; `pawprint autostart on` turns it back on)",
                    );
                }
                Ok(State::On {
                    entry: entry(),
                    command,
                })
            }
            ERROR_FILE_NOT_FOUND => Ok(State::Off),
            code => Err(fail("read", code)),
        }
    }

    /// Whether the user switched the entry off in Startup apps.
    fn switched_off() -> bool {
        let (key, value) = (wide(APPROVED), wide(VALUE));
        let mut buf = [0u8; 64];
        let mut bytes = buf.len() as u32;
        // SAFETY: `buf` holds `bytes` bytes.
        let code = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_BINARY,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        code == ERROR_SUCCESS && bytes > 0 && buf[0] & 1 == 1
    }

    /// Forget the Startup apps switch, so the entry counts as on.
    fn clear_switch() {
        let (key, value) = (wide(APPROVED), wide(VALUE));
        // SAFETY: NUL-terminated strings; a missing value is fine.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
    }

    pub fn enable() -> Result<State, String> {
        let (exe, arg) = super::command_line()?;
        let command = wide(&format!("\"{}\" {arg}", exe.display()));
        let (key, value) = (wide(KEY), wide(VALUE));
        // SAFETY: `command` is NUL-terminated and its size is given in bytes.
        let code = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        };
        if code != ERROR_SUCCESS {
            return Err(fail("write", code));
        }
        clear_switch();
        status()
    }

    pub fn disable() -> Result<(), String> {
        let (key, value) = (wide(KEY), wide(VALUE));
        clear_switch();
        // SAFETY: NUL-terminated strings.
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            code => Err(fail("remove", code)),
        }
    }
}

#[cfg(target_os = "macos")]
use launchd as platform;

/// macOS: `~/Library/LaunchAgents/io.github.nealhardesty.pawprint.plist`,
/// loaded into this login's `gui/<uid>` domain at once and by launchd at
/// every GUI login (`LimitLoadToSessionType = Aqua`, F16). launchd restarts
/// a crashed badge but not one that quit (`KeepAlive = { SuccessfulExit =
/// false }`); `pawprint run` exits 0 when a badge already runs.
#[cfg(target_os = "macos")]
mod launchd {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use super::State;

    const LABEL: &str = "io.github.nealhardesty.pawprint";

    fn plist_path() -> Result<PathBuf, String> {
        let base = directories::BaseDirs::new().ok_or("cannot find the home directory")?;
        Ok(base
            .home_dir()
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn unescape(s: &str) -> String {
        s.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }

    pub fn plist(args: &[&str]) -> String {
        let args: String = args
            .iter()
            .map(|a| format!("\n        <string>{}</string>", escape(a)))
            .collect();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>{args}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
        )
    }

    /// `ProgramArguments` from a plist this module wrote, as one line.
    pub fn command(text: &str) -> String {
        let array = text
            .split_once("<key>ProgramArguments</key>")
            .and_then(|(_, rest)| rest.split_once("</array>"))
            .map_or("", |(array, _)| array);
        let args: Vec<String> = array
            .split("<string>")
            .skip(1)
            .filter_map(|s| s.split_once("</string>"))
            .map(|(arg, _)| unescape(arg))
            .map(|a| {
                if a.contains(char::is_whitespace) {
                    format!("\"{a}\"")
                } else {
                    a
                }
            })
            .collect();
        if args.is_empty() {
            "(no ProgramArguments)".into()
        } else {
            args.join(" ")
        }
    }

    fn domain() -> String {
        // SAFETY: getuid has no preconditions.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    fn launchctl(args: &[&str]) -> Result<Output, String> {
        Command::new("/bin/launchctl")
            .args(args)
            .output()
            .map_err(|e| format!("cannot run launchctl: {e}"))
    }

    /// Whether launchd has the agent loaded in this login.
    fn loaded() -> bool {
        launchctl(&["print", &format!("{}/{LABEL}", domain())]).is_ok_and(|o| o.status.success())
    }

    pub fn status() -> Result<State, String> {
        let path = plist_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(State::On {
                command: format!(
                    "{}{}",
                    command(&text),
                    if loaded() {
                        ""
                    } else {
                        " (not loaded now; launchd loads it at the next login)"
                    }
                ),
                entry: path,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::Off),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Write the agent and load it now, which starts a badge (or finds the
    /// running one and exits). An agent that is already loaded stays as it
    /// is unless its plist changed (a new install path), which launchd only
    /// reads on load, so it is unloaded and loaded again. One that can't
    /// load now (an SSH login) still loads at the next GUI login.
    pub fn enable() -> Result<State, String> {
        let (exe, arg) = super::command_line()?;
        let path = plist_path()?;
        let changed = write(&path, &exe, &arg)?;
        let mut load = !loaded();
        if !load && changed {
            run_logged(&["bootout", &format!("{}/{LABEL}", domain())])?;
            load = true;
        }
        if load {
            run_logged(&["bootstrap", &domain(), &path.to_string_lossy()])?;
        }
        status()
    }

    /// Run launchctl; a failure is logged, not fatal.
    fn run_logged(args: &[&str]) -> Result<(), String> {
        let out = launchctl(args)?;
        if !out.status.success() {
            log::warn!(
                "launchctl {}: {}",
                args[0],
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// Write the plist; true when its content changed.
    fn write(path: &Path, exe: &Path, arg: &str) -> Result<bool, String> {
        let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let text = plist(&[&exe.to_string_lossy(), arg]);
        if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
            return Ok(false);
        }
        std::fs::write(path, text).map_err(fail)?;
        Ok(true)
    }

    /// Unload the agent (which stops a badge it started) and remove it.
    pub fn disable() -> Result<(), String> {
        if loaded() {
            launchctl(&["bootout", &format!("{}/{LABEL}", domain())])?;
        }
        let path = plist_path()?;
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("cannot remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn plist_round_trips_its_command() {
            let text = plist(&["/Users/a b/bin/paw&print", "run"]);
            assert!(text.contains("<string>/Users/a b/bin/paw&amp;print</string>"));
            assert!(text.contains("<key>LimitLoadToSessionType</key>\n    <string>Aqua</string>"));
            assert_eq!(command(&text), "\"/Users/a b/bin/paw&print\" run");
        }
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
        assert_eq!(quote("/opt/100%/pawprint"), "/opt/100%%/pawprint");
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
        // A desktop that hides the entry has turned it off.
        std::fs::write(&path, format!("{text}Hidden=true\n")).unwrap();
        assert_eq!(status_at(&path), Ok(State::Off));
        disable_at(&path).unwrap();
        disable_at(&path).unwrap(); // already off is fine
        assert_eq!(status_at(&path), Ok(State::Off));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
