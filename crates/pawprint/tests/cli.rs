//! The command line as a shell user meets it, without a display.

#![cfg(unix)]

mod common;

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output, Stdio};

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

/// `pawprint update --check` against `base`, in a home of its own: its log
/// goes there, and no badge on this desktop hears about it.
fn update_check(tag: &str, base: &str) -> (Output, String) {
    let home = std::path::PathBuf::from(format!("/tmp/pp-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pawprint"))
        .args(["update", "--check"])
        .env("HOME", &home)
        .env("TMPDIR", &home)
        .env("XDG_RUNTIME_DIR", &home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env("PAWPRINT_DOWNLOAD_BASE", base)
        .output()
        .unwrap();
    let log = ["state/pawprint", "Library/Logs/pawprint"]
        .iter()
        .find_map(|dir| std::fs::read_to_string(home.join(dir).join("pawprint.log")).ok())
        .unwrap_or_default();
    std::fs::remove_dir_all(&home).unwrap();
    (out, log)
}

/// The embedded installer finds the latest release through the redirect,
/// and the answer depends on this build's version.
#[test]
fn update_check_compares_the_latest_release_with_this_build() {
    let this = env!("CARGO_PKG_VERSION");
    let text = |o: &Output| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };

    let (out, log) = update_check("same", &common::release_server(this));
    assert!(out.status.success(), "{}", text(&out));
    let latest = format!("PAWPRINT {this} is the latest release.");
    assert_eq!(text(&out).trim(), latest);
    assert!(log.contains(&latest), "the log says so too: {log}");

    // An older release (a build ahead of the last release) is no update.
    let (out, _) = update_check("older", &common::release_server("0.0.1"));
    assert_eq!(text(&out).trim(), latest);

    let (out, _) = update_check("newer", &common::release_server("99.0.0"));
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).starts_with(&format!("PAWPRINT 99.0.0 is available (this is {this}).")),
        "{}",
        text(&out)
    );
}

#[test]
fn update_check_fails_cleanly_when_offline_or_confused() {
    // Nothing listens on the discard port.
    let (out, log) = update_check("offline", "http://127.0.0.1:9/releases");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("pawprint: cannot find the latest release"),
        "{stderr}"
    );
    assert!(log.contains("cannot find the latest release"), "{log}");

    // A tag that is no version is not installed blindly.
    let (out, _) = update_check("odd", &common::release_server("nightly"));
    assert_eq!(out.status.code(), Some(1));
}
