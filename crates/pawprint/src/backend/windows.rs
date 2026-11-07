//! Windows backend (SPEC §11.1): a layered, topmost, click-through tool
//! window. Not implemented yet; see the M1 handoff in ROADMAP.md.

use crate::shell::Shell;
use crate::{CmdResult, doctor::Report, exit, ipc};

pub fn run(_shell: Shell, _listener: ipc::Listener) -> CmdResult {
    Err((
        exit::NO_SESSION,
        "the Windows backend is not implemented yet (M1)".into(),
    ))
}

pub fn doctor(report: &mut Report) {
    report.section("Windows");
    report.row("backend", "windows (not implemented yet, M1)");
}
