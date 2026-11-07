//! macOS backend (SPEC §11.2): a borderless, non-activating `NSPanel` at
//! status level. Not implemented yet; see the M1 handoff in ROADMAP.md.

use crate::shell::Shell;
use crate::{CmdResult, doctor::Report, exit, ipc};

pub fn run(_shell: Shell, _listener: ipc::Listener) -> CmdResult {
    Err((
        exit::NO_SESSION,
        "the macOS backend is not implemented yet (M1)".into(),
    ))
}

pub fn doctor(report: &mut Report) {
    report.section("macOS");
    report.row("backend", "macos (not implemented yet, M1)");
}
