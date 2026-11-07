//! The command line as a shell user meets it, without a display.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

/// `pawprint explain | head -1`: when the reader goes away, the command
/// ends quietly by SIGPIPE, like any Unix tool, instead of panicking.
#[test]
fn a_closed_pipe_ends_output_quietly() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = Command::new(env!("CARGO_BIN_EXE_pawprint"))
        .arg("list")
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(out.status.signal(), Some(libc::SIGPIPE), "{:?}", out.status);
}
