//! One badge per graphical session (SPEC §10.4): the session key, the IPC
//! endpoint that doubles as the instance lock, and detached startup.
//!
//! Only [`session_key`] and the endpoint directory differ per platform.

use std::io;
use std::path::PathBuf;
#[cfg(unix)]
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

/// macOS: the audit session of the GUI login this process belongs to
/// (`SessionGetInfo`), so the console and each Screen Sharing login get their
/// own badge. A terminal inherits its login's session, so `pawprint move`
/// finds the badge; an SSH login is a separate session without the window
/// server.
#[cfg(target_os = "macos")]
pub fn session_key() -> Result<String, NoSession> {
    match security_session() {
        Some(s) if s.graphic => Ok(format!("asid-{}", s.id)),
        Some(s) => Err(format!(
            "no graphical session: audit session {} has no window server access \
             (an SSH login?). Run PAWPRINT from a Terminal on the desktop; \
             `pawprint doctor` explains more.",
            s.id
        )),
        None => Err("cannot read the security session (SessionGetInfo failed)".into()),
    }
}

/// This process's security session, as `SessionGetInfo` reports it.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
pub struct SecuritySession {
    /// The audit session id.
    pub id: u32,
    /// The session reaches the window server (a GUI login).
    pub graphic: bool,
    /// A remote login (Screen Sharing).
    pub remote: bool,
}

#[cfg(target_os = "macos")]
pub fn security_session() -> Option<SecuritySession> {
    // Security.framework's AuthSession.h, which no binding crate covers.
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SessionGetInfo(session: u32, session_id: *mut u32, attributes: *mut u32) -> i32;
    }
    const CALLER_SECURITY_SESSION: u32 = u32::MAX;
    const HAS_GRAPHIC_ACCESS: u32 = 0x0010;
    const IS_REMOTE: u32 = 0x1000;
    let (mut id, mut attributes) = (0, 0);
    // SAFETY: both out-pointers are valid for writes.
    let status = unsafe { SessionGetInfo(CALLER_SECURITY_SESSION, &mut id, &mut attributes) };
    (status == 0).then_some(SecuritySession {
        id,
        graphic: attributes & HAS_GRAPHIC_ACCESS != 0,
        remote: attributes & IS_REMOTE != 0,
    })
}

/// Windows: `"<user>-<session id>"` from `ProcessIdToSessionId`, so the
/// console and each RDP session get their own badge. The endpoint becomes
/// `\\.\pipe\pawprint-<user>-<session id>`.
#[cfg(windows)]
pub fn session_key() -> Result<String, NoSession> {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
    let mut sid = 0;
    let mut user = [0u16; 257];
    let mut len = user.len() as u32;
    // SAFETY: both calls write only into the buffers given, and `len` holds
    // the buffer size in UTF-16 units.
    let ok = unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut sid) != 0
            && GetUserNameW(user.as_mut_ptr(), &mut len) != 0
    };
    if !ok {
        return Err(format!(
            "cannot read the Windows session: {}",
            io::Error::last_os_error()
        ));
    }
    // `len` counts the terminating NUL. Pipe names may not contain `\`.
    let user = String::from_utf16_lossy(&user[..len.saturating_sub(1) as usize]);
    Ok(format!("{}-{sid}", user.replace('\\', "_")))
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
        None if cfg!(target_os = "macos") => user_temp_dir().join("pawprint"),
        // SAFETY: getuid has no preconditions.
        None => std::env::temp_dir().join(format!("pawprint-{}", unsafe { libc::getuid() })),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn user_temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// macOS: `$TMPDIR`, else the per-user temporary directory it normally names,
/// so a badge started by launchd without `TMPDIR` and a terminal agree.
#[cfg(target_os = "macos")]
fn user_temp_dir() -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    if std::env::var_os("TMPDIR").is_some_and(|d| !d.is_empty()) {
        return std::env::temp_dir();
    }
    let mut buf = [0u8; 1024];
    // SAFETY: buf is valid for buf.len() bytes; confstr NUL-terminates it.
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    match n {
        1..=1024 => PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..n - 1])),
        _ => std::env::temp_dir(),
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
    match ipc::call(endpoint, Request::Status) {
        Ok(ipc::Response {
            status: Some(status),
            ..
        }) => return Ok(Claim::Running(Box::new(status))),
        // Another user holds the name (Windows): not ours to replace.
        Err(CallError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
        _ => {}
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

/// The executable that runs the badge: this one, or on Windows the
/// GUI-subsystem twin next to it, so no console window opens (SPEC
/// §11.1.6). Detached starts and autostart entries both use it.
pub fn badge_exe() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    #[cfg(windows)]
    let exe = Some(exe.with_file_name("pawprintw.exe"))
        .filter(|w| w.exists())
        .unwrap_or(exe);
    Ok(exe)
}

/// A badge started detached, to tell whether it exited early.
#[cfg(unix)]
struct Started(std::process::Child);

/// Start the badge detached from this terminal: a new session with no
/// controlling terminal, null stdio, and none of our other descriptors.
#[cfg(unix)]
fn start_detached() -> io::Result<Started> {
    detached(Command::new(badge_exe()?).arg("run"))
        .spawn()
        .map(Started)
}

#[cfg(unix)]
impl Started {
    fn exit_code(&mut self) -> Option<i32> {
        self.0.try_wait().ok()?.map(|s| s.code().unwrap_or(1))
    }
}

/// `cmd` set up to run detached: `setsid`, null stdio, and every
/// descriptor from 3 up closed at exec. A parent that captures our output
/// through an extra pipe (or a make jobserver) would otherwise wait for the
/// badge to quit, the Unix side of FW8.
#[cfg(unix)]
fn detached(cmd: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid, syscall, sysconf, and fcntl are async-signal-safe and
    // touch no Rust state. Marking the descriptors close-on-exec, rather
    // than closing them, keeps std's own exec-error pipe working.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            if libc::syscall(
                libc::SYS_close_range,
                3u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            ) == 0
            {
                return Ok(());
            }
            let max = match libc::sysconf(libc::_SC_OPEN_MAX) {
                n if n > 0 => n.min(65536) as libc::c_int,
                _ => 1024,
            };
            for fd in 3..max {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        })
    }
}

/// A badge started detached, to tell whether it exited early.
#[cfg(windows)]
struct Started(std::os::windows::io::OwnedHandle);

/// Start the badge with `CreateProcessW` and handle inheritance off, so it
/// holds none of our handles: a redirected stdout it kept open would make
/// `pawprint | more` or a capturing script wait until the badge quits
/// (FW8). `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`: no console, and no
/// Ctrl+C from the terminal that started it. Out of our job if the job
/// allows it, so closing a job-bound terminal doesn't take the badge along.
#[cfg(windows)]
fn start_detached() -> io::Result<Started> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CreateProcessW, DETACHED_PROCESS,
        PROCESS_INFORMATION, STARTUPINFOW,
    };
    let exe = badge_exe()?;
    let line: Vec<u16> = std::ffi::OsStr::new("\"")
        .encode_wide()
        .chain(exe.as_os_str().encode_wide())
        .chain("\" run".encode_utf16())
        .chain([0])
        .collect();
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    let mut last = None;
    for flags in [base | CREATE_BREAKAWAY_FROM_JOB, base] {
        let mut line = line.clone();
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: `line` is a writable NUL-terminated command line; the
        // structs are valid for the call. bInheritHandles = FALSE.
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                std::ptr::null(),
                std::ptr::null(),
                &startup,
                &mut info,
            )
        };
        if ok != 0 {
            // SAFETY: both handles are ours; only the process one is kept.
            unsafe { CloseHandle(info.hThread) };
            return Ok(Started(unsafe {
                std::os::windows::io::OwnedHandle::from_raw_handle(info.hProcess)
            }));
        }
        // A job without breakaway rights refuses the first try.
        last = Some(io::Error::last_os_error());
    }
    Err(last.expect("tried at least once"))
}

#[cfg(windows)]
impl Started {
    fn exit_code(&mut self) -> Option<i32> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        let mut code = 0;
        // SAFETY: a process handle we own.
        let exited = unsafe {
            WaitForSingleObject(self.0.as_raw_handle(), 0) == WAIT_OBJECT_0
                && GetExitCodeProcess(self.0.as_raw_handle(), &mut code) != 0
        };
        exited.then_some(code as i32)
    }
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
    let mut child =
        start_detached().map_err(|e| (exit::ERROR, format!("cannot start the badge: {e}")))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(s)) = running(&endpoint) {
            println!(
                "PAWPRINT started (pid {}, {} backend). `pawprint quit` stops it.",
                s.pid, s.backend
            );
            return Ok(());
        }
        if let Some(code) = child.exit_code() {
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

    /// A detached child keeps none of our descriptors: a parent reading a
    /// pipe we inherited sees end of file once we close our end, while the
    /// child still runs.
    #[test]
    fn detached_children_inherit_no_descriptors() {
        let mut fds = [0; 2];
        // SAFETY: `fds` has room for both ends; plain pipe(), so both ends
        // are inheritable, like a capturing parent's extra pipe.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut child = detached(Command::new("sleep").arg("5")).spawn().unwrap();
        let mut poll = libc::pollfd {
            fd: fds[0],
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: our own descriptors; `poll` and `byte` are valid.
        let (ready, n) = unsafe {
            libc::close(fds[1]);
            let ready = libc::poll(&mut poll, 1, 2000);
            let mut byte = 0u8;
            let n = libc::read(fds[0], (&raw mut byte).cast(), 1);
            libc::close(fds[0]);
            (ready, n)
        };
        assert!(child.try_wait().unwrap().is_none(), "the child still runs");
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!((ready, n), (1, 0), "end of file, not a pipe held open");
    }
}
