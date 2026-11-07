//! `pawprint`: the badge and its command-line interface (SPEC §9).
//!
//! The code lives in this library so that two executables share one
//! [`main`]: `pawprint` (`src/main.rs`) and, on Windows, the GUI-subsystem
//! `pawprintw` (`src/bin/pawprintw.rs`, SPEC §11.1.6).

// The Windows and macOS backends are stubs until they land (M1), so the
// shared runtime they will drive is unused there. Remove with them.

mod backend;
mod clock;
mod commands;
mod doctor;
mod hostname;
mod instance;
mod ipc;
mod logger;
mod paths;
mod shell;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

/// Exit codes (SPEC §9).
pub mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const NO_SESSION: u8 = 3;
    pub const NOT_RUNNING: u8 = 4;
}

#[derive(Parser)]
#[command(
    name = "pawprint",
    about = "A tiny pixel-art animal in the corner of your screen, stamped with this machine's name.",
    after_help = "Run `pawprint` with no command to start the badge. `pawprint explain` shows why it looks the way it does."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the badge in the foreground (autostart uses this)
    Run {
        /// Log to stderr
        #[arg(long)]
        verbose: bool,
    },
    /// Enter move mode on the running badge
    Move,
    /// Forget the dragged position
    ResetPosition,
    /// Re-read config.toml
    Reload,
    /// Stop the running badge
    Quit,
    /// Is it running? Backend, display, mascot, label, position
    Status,
    /// Show how the label, mascot, and every setting were chosen
    Explain {
        /// Explain for this hostname instead of this machine's
        #[arg(long, value_name = "NAME")]
        host: Option<String>,
        /// Ignore the cached fallback mascot and show what the hash picks today
        #[arg(long)]
        recompute: bool,
    },
    /// List mascots, or the accent palette
    List {
        /// Only this category (mammals, cats-dogs, birds, aquatic, critters, oddballs)
        #[arg(long, value_name = "C")]
        category: Option<String>,
        /// List the accent palette instead
        #[arg(long)]
        accents: bool,
    },
    /// Open a window showing a mascot cycling through its actions
    Preview {
        id: String,
        #[arg(long, value_name = "N", default_value_t = 128, value_parser = parse_size)]
        size: u32,
    },
    /// Write a badge image (PNG)
    Render {
        /// Mascot id (see `pawprint list`)
        id: String,
        /// Output file
        #[arg(long, value_name = "FILE.png")]
        out: PathBuf,
        /// Mascot size in pixels: 32, 64, 128, or any multiple of 32
        #[arg(long, value_name = "N", default_value_t = 64, value_parser = parse_size)]
        size: u32,
        /// Label text [default: this machine's label]; "none" hides it
        #[arg(long, value_name = "TEXT")]
        label: Option<String>,
        /// Frame number (0 is the rest frame)
        #[arg(long, value_name = "N", default_value_t = 0)]
        frame: usize,
    },
    /// Set a key in this host's [hosts] table, then reload
    Set { key: String, value: String },
    /// Remove a key from this host's [hosts] table
    Unset { key: String },
    /// Drop this host's cached fallback mascot
    Forget,
    /// Config file helpers
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Custom mascot helpers
    #[command(subcommand)]
    Mascot(MascotCmd),
    /// Start at graphical login: on, off, or status
    Autostart { action: Toggle },
    /// Diagnose the desktop session and backend
    Doctor,
    /// Print every file location
    Paths,
    /// GNOME presenter extension (only if needed): install or remove
    GnomeExtension { action: InstallRemove },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Write a commented config.toml if none exists
    Init,
    /// Print the config file path
    Path,
}

#[derive(Subcommand)]
enum MascotCmd {
    /// Write a bundled mascot as a PNG strip plus mascot.toml
    Export { id: String, dir: PathBuf },
    /// Validate a custom mascot directory
    Check { dir: PathBuf },
}

#[derive(Clone, ValueEnum)]
enum Toggle {
    On,
    Off,
    Status,
}

#[derive(Clone, ValueEnum)]
enum InstallRemove {
    Install,
    Remove,
}

fn parse_size(s: &str) -> Result<u32, String> {
    match s.parse::<u32>() {
        Ok(n) if n > 0 && n % 32 == 0 && n <= 1024 => Ok(n),
        _ => Err("must be a positive multiple of 32 (32, 64, 128, ...)".into()),
    }
}

/// `pawprint 0.1.0 (commit, target, N mascots)`
fn version() -> String {
    let n = pawprint_core::catalog::Catalog::bundled().mascots().len();
    format!(
        "{} ({}, {}, {n} mascot{})",
        env!("CARGO_PKG_VERSION"),
        env!("PAWPRINT_GIT_COMMIT"),
        env!("PAWPRINT_TARGET"),
        if n == 1 { "" } else { "s" }
    )
}

/// The whole program; both executables call this.
pub fn main() -> ExitCode {
    let version: &'static str = Box::leak(version().into_boxed_str());
    let matches = Cli::command().version(version).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    let result = match cli.command {
        None => instance::start(),
        Some(Cmd::Run { verbose }) => run(verbose),
        Some(Cmd::Move) => {
            // Let the badge take the keyboard from this terminal.
            #[cfg(windows)]
            backend::allow_foreground();
            send(ipc::Request::Move).map(|_| {
                println!(
                    "Move mode: drag the badge, or use the arrow keys (Shift for bigger steps). \
                     Enter saves, Esc cancels; it ends by itself after 30 seconds."
                )
            })
        }
        Some(Cmd::ResetPosition) => send(ipc::Request::ResetPosition)
            .map(|_| println!("Position reset to the configured one.")),
        Some(Cmd::Reload) => send(ipc::Request::Reload).map(|_| println!("Config reloaded.")),
        Some(Cmd::Quit) => send(ipc::Request::Quit).map(|_| println!("PAWPRINT stopped.")),
        Some(Cmd::Status) => status(),
        Some(Cmd::Doctor) => doctor::run(),
        Some(Cmd::Preview { .. }) => not_yet("preview", "M2"),
        Some(Cmd::Set { .. }) => not_yet("set", "M4"),
        Some(Cmd::Unset { .. }) => not_yet("unset", "M4"),
        Some(Cmd::Forget) => not_yet("forget", "M4"),
        Some(Cmd::Config(ConfigCmd::Init)) => not_yet("config init", "M4"),
        Some(Cmd::Config(ConfigCmd::Path)) => commands::config_path(),
        Some(Cmd::Mascot(MascotCmd::Export { id, dir })) => commands::mascot_export(&id, &dir),
        Some(Cmd::Mascot(MascotCmd::Check { dir })) => commands::mascot_check(&dir),
        Some(Cmd::Autostart { .. }) => not_yet("autostart", "M4"),
        Some(Cmd::GnomeExtension { .. }) => not_yet("gnome-extension", "M1, only if needed"),
        Some(Cmd::Explain { host, recompute }) => commands::explain(host, recompute),
        Some(Cmd::List { category, accents }) => commands::list(category, accents),
        Some(Cmd::Render {
            id,
            out,
            size,
            label,
            frame,
        }) => commands::render(&id, &out, size, label, frame),
        Some(Cmd::Paths) => commands::paths(),
    };
    match result {
        Ok(()) => ExitCode::from(exit::OK),
        Err((code, msg)) => {
            eprintln!("pawprint: {msg}");
            ExitCode::from(code)
        }
    }
}

/// A command failure: exit code and message.
pub type CmdResult = Result<(), (u8, String)>;

/// `pawprint run`: the badge, in the foreground.
fn run(verbose: bool) -> CmdResult {
    let paths = paths::Paths::detect()
        .ok_or((exit::ERROR, "cannot find the home directory".to_string()))?;
    logger::init(&paths.log_file, verbose);
    let fail = |code: u8, msg: String| {
        log::error!("{msg}");
        (code, msg)
    };
    let session = instance::session_key().map_err(|e| fail(exit::NO_SESSION, e))?;
    let endpoint = instance::endpoint().map_err(|e| fail(exit::NO_SESSION, e))?;
    let listener = match instance::claim(&endpoint) {
        Ok(instance::Claim::Claimed(l)) => l,
        Ok(instance::Claim::Running(s)) => {
            println!(
                "PAWPRINT is already running in this session (pid {}).",
                s.pid
            );
            return Ok(());
        }
        Err(e) => {
            return Err(fail(
                exit::ERROR,
                format!("cannot open {}: {e}", endpoint.display()),
            ));
        }
    };
    log::info!(
        "PAWPRINT {} starting: pid {}, session {session}",
        version(),
        std::process::id()
    );
    let result = shell::Shell::new(paths, session, Default::default())
        .and_then(|shell| backend::run(shell, listener));
    instance::release(&endpoint);
    result.map_err(|(code, msg)| fail(code, msg))
}

/// Send one command to this session's badge.
fn send(request: ipc::Request) -> Result<ipc::Response, (u8, String)> {
    let endpoint = instance::endpoint().map_err(|e| (exit::NO_SESSION, e))?;
    match ipc::call(&endpoint, request) {
        Ok(r) if r.ok => Ok(r),
        Ok(r) => Err((exit::ERROR, r.error.unwrap_or_else(|| "failed".into()))),
        Err(ipc::CallError::NotRunning) => Err((
            exit::NOT_RUNNING,
            "PAWPRINT is not running in this session; start it with `pawprint`".into(),
        )),
        Err(e) => Err((exit::ERROR, e.to_string())),
    }
}

fn status() -> CmdResult {
    let s = send(ipc::Request::Status)?
        .status
        .ok_or((exit::ERROR, "the badge sent no status".to_string()))?;
    let rows = [
        ("running", format!("yes, pid {}", s.pid)),
        ("backend", s.backend),
        ("session", s.session),
        ("display", s.display.unwrap_or_else(|| "(none)".into())),
        (
            "badge",
            match (s.rect, s.scale) {
                (Some([x, y, w, h]), Some(scale)) => format!("{w}x{h} at {x},{y} (scale {scale})"),
                _ => "(not placed)".into(),
            },
        ),
        ("mascot", s.mascot),
        ("label", s.label.unwrap_or_else(|| "(none)".into())),
        ("size", s.size.to_string()),
        (
            "position",
            format!("{} ({})", s.position, s.position_source),
        ),
        ("move mode", if s.moving { "on" } else { "off" }.into()),
    ];
    for (k, v) in rows {
        println!("{k:<12}{v}");
    }
    Ok(())
}

fn not_yet(what: &str, milestone: &str) -> CmdResult {
    Err((
        exit::ERROR,
        format!("{what}: not implemented yet ({milestone}); see ROADMAP.md"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn size_parser() {
        assert_eq!(parse_size("64"), Ok(64));
        assert_eq!(parse_size("96"), Ok(96));
        for bad in ["0", "50", "-32", "big", "2048"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }
}
