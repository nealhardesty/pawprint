//! Where PAWPRINT keeps its files (SPEC §8.1).

use std::path::PathBuf;

use directories::BaseDirs;

const APP: &str = "pawprint";

pub struct Paths {
    pub config_file: PathBuf,
    pub user_mascots: PathBuf,
    pub state_file: PathBuf,
    pub log_file: PathBuf,
    /// The IPC endpoint pattern; the session part is filled in by `run` (M1).
    pub ipc: String,
    pub binary: PathBuf,
}

impl Paths {
    pub fn detect() -> Option<Paths> {
        let base = BaseDirs::new()?;
        let config_dir = base.config_dir().join(APP);
        let home = base.home_dir();
        let (state_dir, log_file, ipc, binary);
        if cfg!(windows) {
            state_dir = base.data_local_dir().join(APP);
            log_file = state_dir.join("pawprint.log");
            ipc = r"\\.\pipe\pawprint-<user>-<session-id>".to_string();
            binary = base
                .data_local_dir()
                .join("Programs")
                .join(APP)
                .join("pawprint.exe");
        } else if cfg!(target_os = "macos") {
            state_dir = config_dir.clone();
            log_file = home.join("Library/Logs/pawprint/pawprint.log");
            let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
            ipc = PathBuf::from(tmp)
                .join(APP)
                .join("<session>.sock")
                .display()
                .to_string();
            binary = home.join(".local/bin/pawprint");
        } else {
            state_dir = base
                .state_dir()
                .map_or_else(|| home.join(".local/state"), PathBuf::from)
                .join(APP);
            log_file = state_dir.join("pawprint.log");
            ipc = match base.runtime_dir() {
                Some(dir) => dir.join(APP).join("<session>.sock").display().to_string(),
                None => {
                    "$XDG_RUNTIME_DIR/pawprint/<session>.sock ($XDG_RUNTIME_DIR is not set)".into()
                }
            };
            binary = home.join(".local/bin/pawprint");
        }
        Some(Paths {
            config_file: config_dir.join("config.toml"),
            user_mascots: config_dir.join("mascots"),
            state_file: state_dir.join("state.toml"),
            log_file,
            ipc,
            binary,
        })
    }
}
