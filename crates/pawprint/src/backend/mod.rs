//! Window backends (SPEC §11). Each one owns a native loop and a
//! `Surface`, translates native events for the [`Shell`], and serves IPC
//! through `ipc::serve`. Only the backends for the build's OS are compiled.

use crate::shell::Shell;
use crate::{CmdResult, doctor::Report, ipc};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod menubar;
#[cfg(target_os = "linux")]
mod sni;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_tray;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

/// Run the badge until it quits. `listener` is this session's claimed IPC
/// endpoint (SPEC §10.4).
pub fn run(shell: Shell, listener: ipc::Listener) -> CmdResult {
    platform::run(shell, listener)
}

/// `pawprint preview`: a normal window showing `shell`'s mascot until it is
/// closed. Backends reuse their badge surface with a preview flag
/// (`Shell::is_preview`).
pub fn preview(shell: Shell) -> CmdResult {
    platform::preview(shell)
}

/// Add the platform's checks to `pawprint doctor`.
pub fn doctor(report: &mut Report) {
    platform::doctor(report);
}
