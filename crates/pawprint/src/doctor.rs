//! `pawprint doctor` (SPEC §9): the shared checks, then the platform's own
//! (`backend::doctor`).

use pawprint_core::config::{Layer, Settings};
use pawprint_core::surface::{DisplayInfo, select_display};

use crate::paths::Paths;
use crate::{CmdResult, backend, commands, instance, shell};

/// What doctor prints: aligned rows under section headings, then problems,
/// each with what to do about it.
#[derive(Default)]
pub struct Report {
    lines: Vec<String>,
    problems: Vec<String>,
}

impl Report {
    pub fn section(&mut self, title: &str) {
        self.lines.push(format!("\n{title}"));
    }

    pub fn row(&mut self, key: &str, value: impl AsRef<str>) {
        self.lines.push(format!("  {key:<20}{}", value.as_ref()));
    }

    /// Every display, with the one the badge goes on (SPEC §6.2, from this
    /// host's `display` setting) marked.
    pub fn displays(&mut self, displays: &[DisplayInfo]) {
        if displays.is_empty() {
            self.row("displays", "none found");
        }
        let target = select_display(displays, &settings().display.value);
        for d in displays {
            let mark = if Some(d) == target { "  <- badge" } else { "" };
            self.row("display", format!("{}{mark}", shell::describe(d)));
        }
    }

    /// Something is wrong or degraded; say how to fix it.
    pub fn problem(&mut self, text: impl Into<String>) {
        self.problems.push(text.into());
    }

    fn print(&self) {
        println!("PAWPRINT doctor");
        for l in &self.lines {
            println!("{l}");
        }
        if self.problems.is_empty() {
            println!("\nNo problems found.");
        } else {
            println!("\nProblems");
            for p in &self.problems {
                println!("  ! {p}");
            }
        }
    }
}

/// This host's effective settings from the config file, as the badge
/// resolves them (host tables included), for the platform checks.
pub fn settings() -> Settings {
    let short = pawprint_core::host::short_name(&crate::hostname::detect().unwrap_or_default());
    Paths::detect()
        .map(|p| commands::load_config(&p).0)
        .unwrap_or_default()
        .effective(&short, &Layer::default())
}

pub fn run() -> CmdResult {
    let mut report = Report::default();
    report.row("version", crate::version());
    match crate::hostname::detect() {
        Ok(h) => report.row("hostname", h),
        Err(e) => report.problem(e),
    }

    report.section("Files");
    if let Some(paths) = Paths::detect() {
        let (config, note) = commands::load_config(&paths);
        report.row("config", note);
        for w in &config.warnings {
            report.problem(format!("config: {w}"));
        }
        report.row("state", paths.state_file.display().to_string());
        report.row("log", paths.log_file.display().to_string());
        match crate::autostart::status() {
            Ok(s) => report.row("autostart", crate::autostart::describe(&s)),
            Err(e) => report.row("autostart", e),
        }
    } else {
        report.problem("cannot find the home directory");
    }

    report.section("Instance");
    match instance::endpoint() {
        Ok(endpoint) => {
            report.row("ipc", endpoint.display().to_string());
            match instance::running(&endpoint) {
                Ok(Some(s)) => report.row(
                    "badge",
                    format!("running, pid {}, {} backend", s.pid, s.backend),
                ),
                Ok(None) => report.row("badge", "not running (start it with `pawprint`)"),
                Err(e) => report.problem(e.to_string()),
            }
        }
        Err(e) => report.problem(e),
    }

    backend::doctor(&mut report);
    report.print();
    Ok(())
}
