//! `pawprint update` (SPEC §15.8): ask what the latest release is and
//! install it, with the installer this binary carries. The one command
//! that uses the network, and only when the user asks: from a terminal, or
//! through the tray's update item, which runs this command detached.
//!
//! The installer (`install.sh`, or `install.ps1` on Windows) does all of
//! it, exactly as the install one-liner does: the redirect that names the
//! latest release, the download, the checksum, the trial run of the new
//! binary, the swap, and the restart. The result goes to this session's
//! badge, which says it on the crate.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use pawprint_core::update::{News, Version};

use crate::paths::Paths;
use crate::{CmdResult, exit, instance, ipc, logger};

#[cfg(not(windows))]
const INSTALLER: &str = include_str!("../../../installers/install.sh");
#[cfg(windows)]
const INSTALLER: &str = include_str!("../../../installers/install.ps1");

/// This build's version.
fn current() -> Version {
    env!("CARGO_PKG_VERSION")
        .parse()
        .expect("the workspace version is major.minor.patch")
}

/// `pawprint update [--check]`.
pub fn run(check_only: bool) -> CmdResult {
    // The tray runs this with no terminal: the log is where it can speak.
    if let Some(paths) = Paths::detect() {
        logger::init(&paths.log_file, false);
    }
    let result = update(check_only);
    let news = match &result {
        Ok(news) => *news,
        Err((_, e)) => {
            log::error!("update: {e}");
            News::Failed
        }
    };
    // Best effort: there may be no badge, and a failed install may have
    // stopped it.
    if let Ok(endpoint) = instance::endpoint() {
        let _ = ipc::call(&endpoint, ipc::Request::Update(news));
    }
    result.map(|_| ())
}

fn say(msg: &str) {
    println!("{msg}");
    log::info!("update: {msg}");
}

fn update(check_only: bool) -> Result<News, (u8, String)> {
    let fail = |msg: String| (exit::ERROR, msg);
    let current = current();
    let (ok, text) = installer(&[flag("--latest", "-Latest")]).map_err(fail)?;
    let latest = text.lines().rev().find(|l| !l.trim().is_empty());
    let latest: Version = match latest.map(|l| l.trim().parse()) {
        Some(Ok(v)) if ok => v,
        _ => {
            return Err(fail(format!(
                "cannot find the latest release (is this machine online?): {}",
                first_line(&text)
            )));
        }
    };
    if latest <= current {
        say(&format!("PAWPRINT {current} is the latest release."));
        return Ok(News::UpToDate);
    }
    if check_only {
        say(&format!(
            "PAWPRINT {latest} is available (this is {current}). \
             `pawprint update` installs it, and so does the tray menu's Install update."
        ));
        return Ok(News::Available(latest));
    }

    let dir = install_dir().map_err(fail)?;
    let running = instance::endpoint()
        .ok()
        .and_then(|e| instance::running(&e).ok().flatten())
        .is_some();
    say(&format!(
        "Updating PAWPRINT {current} to {latest} in {}",
        dir.display()
    ));
    let version = latest.to_string();
    // Autostart stays as it is, and only a badge that was running is
    // started again.
    let mut args = vec![
        flag("--version", "-Version"),
        &version,
        flag("--prefix", "-Prefix"),
        dir.to_str()
            .ok_or_else(|| fail(format!("{} is not valid Unicode", dir.display())))?,
        flag("--no-autostart", "-NoAutostart"),
    ];
    if !running {
        args.push(flag("--no-start", "-NoStart"));
    }
    let (ok, text) = installer(&args).map_err(fail)?;
    // After a blank line the installer goes on to `pawprint explain` and
    // PATH advice, which a fresh install wants and an update doesn't.
    text.lines()
        .take_while(|l| !ok || !l.trim().is_empty())
        .for_each(say);
    if !ok {
        return Err(fail(format!(
            "the update to {latest} failed; PAWPRINT {current} is still installed"
        )));
    }
    Ok(News::Updated)
}

fn flag(sh: &'static str, powershell: &'static str) -> &'static str {
    if cfg!(windows) { powershell } else { sh }
}

/// The line of the installer's output that says what went wrong: its own
/// complaint if PowerShell wrapped one in an error record, else the first
/// line (curl's, or the installer's).
fn first_line(text: &str) -> &str {
    let lines = || text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines()
        .find_map(|l| l.split_once("pawprint: ").map(|(_, said)| said))
        .or_else(|| lines().next())
        .unwrap_or("the installer said nothing")
}

/// Where this binary is installed, which is where its replacement goes. A
/// cargo build is refused: the release would land in `target/`, where
/// cargo would keep running it.
fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this executable: {e}"))?;
    // A symlink on PATH is followed to the real file. Not on Windows,
    // where the canonical form is a `\\?\` path PowerShell can't take.
    #[cfg(unix)]
    let exe = exe.canonicalize().unwrap_or(exe);
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no directory", exe.display()))?;
    if dir.join(".cargo-lock").exists() {
        return Err(format!(
            "{} is a development build, which `pawprint update` leaves alone; \
             `make install` installs it",
            exe.display()
        ));
    }
    Ok(dir.to_path_buf())
}

/// Run the embedded installer and return whether it succeeded and what it
/// printed. Its output goes through a file, not a pipe: the badge it
/// starts can inherit the pipe and hold it open until it quits (FW8).
fn installer(args: &[&str]) -> Result<(bool, String), String> {
    let base = std::env::temp_dir().join(format!("pawprint-update-{}", std::process::id()));
    let log = base.with_extension("log");
    let script = base.with_extension("ps1");
    let result = run_installer(args, &log, &script);
    let text = std::fs::read(&log)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(&script);
    result.map(|ok| (ok, text))
}

fn run_installer(args: &[&str], log: &Path, script: &Path) -> Result<bool, String> {
    let out =
        std::fs::File::create(log).map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    let err = out
        .try_clone()
        .map_err(|e| format!("cannot write {}: {e}", log.display()))?;
    let mut cmd = shell(script)?;
    let status = cmd
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", cmd.get_program().to_string_lossy()))?;
    Ok(status.success())
}

/// `sh` with the installer as its script (`$0` is `install.sh`, the
/// arguments follow), as `curl ... | sh -s --` runs it.
#[cfg(not(windows))]
fn shell(_script: &Path) -> Result<Command, String> {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", INSTALLER, "install.sh"]);
    Ok(cmd)
}

/// Windows PowerShell 5.1, which every Windows has, with the installer in
/// a temporary file. No console window opens (the tray's badge has none),
/// and a `PSModulePath` inherited from PowerShell 7 is dropped, since it
/// hides 5.1's own modules.
#[cfg(windows)]
fn shell(script: &Path) -> Result<Command, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::fs::write(script, INSTALLER)
        .map_err(|e| format!("cannot write {}: {e}", script.display()))?;
    let mut cmd = Command::new("powershell.exe");
    cmd.env_remove("PSModulePath")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_has_a_version() {
        assert_eq!(current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_reason_is_found_in_the_installers_output() {
        assert_eq!(
            first_line("\ncurl: (6) Could not resolve host\nerror: cannot find it\n"),
            "curl: (6) Could not resolve host"
        );
        let record = "Exception: C:\\t\\u.ps1:182\nLine |\n 182 | throw ...\n     | pawprint: cannot reach x (timed out); pass -Version\n";
        assert_eq!(
            first_line(record),
            "cannot reach x (timed out); pass -Version"
        );
        assert_eq!(first_line(" \n"), "the installer said nothing");
    }

    #[test]
    fn a_cargo_build_is_never_updated_in_place() {
        // The test binary sits in target/<profile>/deps, which cargo does
        // not lock; `pawprint` itself sits one level up, which it does.
        let deps = std::env::current_exe().unwrap();
        let profile = deps.parent().unwrap().parent().unwrap();
        assert!(profile.join(".cargo-lock").exists(), "cargo's layout");
        assert!(install_dir().is_ok());
    }
}
