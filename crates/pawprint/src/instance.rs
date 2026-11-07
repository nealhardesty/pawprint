//! One badge per graphical session (SPEC §10.4): the session key, the IPC
//! endpoint that doubles as the instance lock, and detached startup.
//!
//! Only [`session_key`] and the endpoint directory differ per platform.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ipc::{self, CallError, Listener, Platform, Request, Status, Transport};
use crate::{CmdResult, exit};

/// Why there is no session key: no graphical session (exit code 3).
pub type NoSession = String;

/// Linux: the Wayland display if there is one, else the X11 display. A
/// terminal in the session sees the same variables as the badge, so `pawprint
/// move` finds the badge without probing any display server. GNOME on
/// Wayland runs the badge through XWayland but still keys on `WAYLAND_DISPLAY`.
#[cfg(target_os = "linux")]
pub fn session_key() -> Result<String, NoSession> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(w) = var("WAYLAND_DISPLAY") {
        // An absolute socket path is allowed; keep only a file-name-safe form.
        return Ok(format!(
            "wayland-{}",
            w.trim_start_matches("wayland-").replace('/', "_")
        ));
    }
    if let Some(d) = var("DISPLAY") {
        return Ok(format!("x11-{}", x11_display_id(&d)));
    }
    Err(
        "no graphical session: neither WAYLAND_DISPLAY nor DISPLAY is set. \
         Run PAWPRINT inside a desktop session; `pawprint doctor` explains more."
            .into(),
    )
}

/// `:0`, `:0.0`, and `unix:0` are the same X display: keep host and display
/// number, drop the screen.
#[cfg(target_os = "linux")]
fn x11_display_id(display: &str) -> String {
    let (host, rest) = display.rsplit_once(':').unwrap_or(("", display));
    let number = rest.split('.').next().unwrap_or(rest);
    let host = if host == "unix" { "" } else { host };
    format!(
        "{}{number}",
        if host.is_empty() {
            String::new()
        } else {
            format!("{host}_")
        }
    )
    .replace('/', "_")
}

/// macOS: the GUI security session (`SessionGetInfo`).
///
/// Hook for the macOS backend: call `SessionGetInfo(callerSecuritySession,
/// ...)` from the Security framework and return the audit session id, so
/// each GUI login (console or Screen Sharing) gets its own badge.
#[cfg(target_os = "macos")]
pub fn session_key() -> Result<String, NoSession> {
    Err("the macOS session key (SessionGetInfo) is not implemented yet (M1)".into())
}

/// Windows: the logon session (`ProcessIdToSessionId`), so each RDP session
/// gets its own badge.
///
/// Hook for the Windows backend: return `"<user>-<session id>"` from
/// `ProcessIdToSessionId(GetCurrentProcessId())` and the user name; the
/// endpoint becomes `\\.\pipe\pawprint-<user>-<session id>`.
#[cfg(windows)]
pub fn session_key() -> Result<String, NoSession> {
    Err("the Windows session key (ProcessIdToSessionId) is not implemented yet (M1)".into())
}

/// The IPC endpoint for this session.
pub fn endpoint() -> Result<PathBuf, NoSession> {
    let key = session_key()?;
    Ok(if cfg!(windows) {
        PathBuf::from(format!(r"\\.\pipe\pawprint-{key}"))
    } else {
        endpoint_dir().join(format!("{key}.sock"))
    })
}

/// Linux: `$XDG_RUNTIME_DIR/pawprint`, falling back to a private directory in
/// /tmp for sessions without one. macOS: `$TMPDIR/pawprint`.
#[cfg(unix)]
fn endpoint_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty() && cfg!(target_os = "linux"))
    {
        Some(dir) => PathBuf::from(dir).join("pawprint"),
        None if cfg!(target_os = "macos") => std::env::temp_dir().join("pawprint"),
        // SAFETY: getuid has no preconditions.
        None => std::env::temp_dir().join(format!("pawprint-{}", unsafe { libc::getuid() })),
    }
}

#[cfg(windows)]
fn endpoint_dir() -> PathBuf {
    PathBuf::new()
}

/// The result of trying to become this session's badge.
pub enum Claim {
    /// We hold the endpoint; serve IPC on it.
    Claimed(Listener),
    /// A live badge already answers on it.
    Running(Box<Status>),
}

/// Take the endpoint. If it exists and a badge answers `status`, that badge
/// keeps it. If nobody answers, it is stale: remove it and take it.
pub fn claim(endpoint: &std::path::Path) -> io::Result<Claim> {
    match Platform::bind(endpoint) {
        Ok(l) => return Ok(Claim::Claimed(l)),
        Err(e) if e.kind() != io::ErrorKind::AddrInUse => return Err(e),
        Err(_) => {}
    }
    if let Ok(ipc::Response {
        status: Some(status),
        ..
    }) = ipc::call(endpoint, Request::Status)
    {
        return Ok(Claim::Running(Box::new(status)));
    }
    log::info!("removing stale endpoint {}", endpoint.display());
    Platform::remove_stale(endpoint)?;
    Platform::bind(endpoint).map(Claim::Claimed)
}

/// Remove our endpoint on a clean exit.
pub fn release(endpoint: &std::path::Path) {
    let _ = Platform::remove_stale(endpoint);
}

/// A running badge's status, if one answers in this session.
pub fn running(endpoint: &std::path::Path) -> Result<Option<Status>, CallError> {
    match ipc::call(endpoint, Request::Status) {
        Ok(r) => Ok(r.status),
        Err(CallError::NotRunning) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The command that runs the badge detached from this terminal: a new
/// session with no controlling terminal and null stdio on Unix, a detached
/// process on Windows.
fn detached_badge() -> io::Result<Command> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches no Rust state.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP. The Windows backend
        // switches this to `pawprintw.exe run` (SPEC §11.1.6).
        cmd.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    Ok(cmd)
}

/// Bare `pawprint`: start the badge in the background for this session and
/// return once it answers.
pub fn start() -> CmdResult {
    let endpoint = endpoint().map_err(|e| (exit::NO_SESSION, e))?;
    match running(&endpoint) {
        Ok(Some(s)) => {
            println!(
                "PAWPRINT is already running in this session (pid {}).",
                s.pid
            );
            return Ok(());
        }
        Ok(None) => {}
        Err(e) => return Err((exit::ERROR, e.to_string())),
    }
    let mut child = detached_badge()
        .and_then(|mut c| c.spawn())
        .map_err(|e| (exit::ERROR, format!("cannot start the badge: {e}")))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(s)) = running(&endpoint) {
            println!(
                "PAWPRINT started (pid {}, {} backend). `pawprint quit` stops it.",
                s.pid, s.backend
            );
            return Ok(());
        }
        if let Ok(Some(code)) = child.try_wait() {
            let code = code.code().unwrap_or(1);
            let hint = crate::paths::Paths::detect()
                .map(|p| format!("see {} or run `pawprint doctor`", p.log_file.display()))
                .unwrap_or_else(|| "run `pawprint doctor`".into());
            return Err((
                u8::try_from(code).unwrap_or(exit::ERROR),
                format!("the badge exited (code {code}); {hint}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("PAWPRINT is starting in the background; `pawprint status` checks on it.");
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn x11_display_ids() {
        assert_eq!(x11_display_id(":0"), "0");
        assert_eq!(x11_display_id(":0.0"), "0");
        assert_eq!(x11_display_id("unix:10.0"), "10");
        assert_eq!(x11_display_id("localhost:10.0"), "localhost_10");
    }
}
