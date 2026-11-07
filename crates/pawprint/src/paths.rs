//! Where PAWPRINT keeps its files (SPEC §8.1).

use std::path::PathBuf;

use directories::BaseDirs;

const APP: &str = "pawprint";

pub struct Paths {
    pub config_file: PathBuf,
    pub user_mascots: PathBuf,
    pub state_file: PathBuf,
    pub log_file: PathBuf,
    /// This session's IPC endpoint, or why there is none.
    pub ipc: String,
    pub binary: PathBuf,
}

impl Paths {
    pub fn detect() -> Option<Paths> {
        let base = BaseDirs::new()?;
        let config_dir = base.config_dir().join(APP);
        let home = base.home_dir();
        let (state_dir, log_file, binary);
        if cfg!(windows) {
            state_dir = base.data_local_dir().join(APP);
            log_file = state_dir.join("pawprint.log");
            binary = base
                .data_local_dir()
                .join("Programs")
                .join(APP)
                .join("pawprint.exe");
        } else if cfg!(target_os = "macos") {
            state_dir = config_dir.clone();
            log_file = home.join("Library/Logs/pawprint/pawprint.log");
            binary = home.join(".local/bin/pawprint");
        } else {
            state_dir = base
                .state_dir()
                .map_or_else(|| home.join(".local/state"), PathBuf::from)
                .join(APP);
            log_file = state_dir.join("pawprint.log");
            binary = home.join(".local/bin/pawprint");
        }
        Some(Paths {
            config_file: config_dir.join("config.toml"),
            user_mascots: config_dir.join("mascots"),
            state_file: state_dir.join("state.toml"),
            log_file,
            ipc: crate::instance::endpoint()
                .map_or_else(|e| format!("(none) {e}"), |p| p.display().to_string()),
            binary,
        })
    }
}
