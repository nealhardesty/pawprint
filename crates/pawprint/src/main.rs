//! `pawprint`: the badge and its command-line interface (SPEC §9).

mod commands;
mod hostname;
mod paths;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

/// Exit codes (SPEC §9).
pub mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const USAGE: u8 = 2;
    #[allow(dead_code)] // used by `run` once backends land (M1)
    pub const NO_SESSION: u8 = 3;
    #[allow(dead_code)]
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

fn main() -> ExitCode {
    let version: &'static str = Box::leak(version().into_boxed_str());
    let matches = Cli::command().version(version).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    let result = match cli.command {
        None => not_yet("starting the badge", "M1"),
        Some(Cmd::Run { .. }) => not_yet("run", "M1"),
        Some(Cmd::Move) => not_yet("move", "M1"),
        Some(Cmd::ResetPosition) => not_yet("reset-position", "M1"),
        Some(Cmd::Reload) => not_yet("reload", "M4"),
        Some(Cmd::Quit) => not_yet("quit", "M1"),
        Some(Cmd::Status) => not_yet("status", "M1"),
        Some(Cmd::Doctor) => not_yet("doctor", "M1"),
        Some(Cmd::Preview { .. }) => not_yet("preview", "M2"),
        Some(Cmd::Set { .. }) => not_yet("set", "M4"),
        Some(Cmd::Unset { .. }) => not_yet("unset", "M4"),
        Some(Cmd::Forget) => not_yet("forget", "M4"),
        Some(Cmd::Config(ConfigCmd::Init)) => not_yet("config init", "M4"),
        Some(Cmd::Config(ConfigCmd::Path)) => commands::config_path(),
        Some(Cmd::Mascot(_)) => not_yet("mascot", "M3"),
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
