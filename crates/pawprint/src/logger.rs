//! The log file (SPEC §8.1): `pawprint.log`, at most 1 MB, one rotation to
//! `pawprint.log.1`. `pawprint run --verbose` also logs to stderr.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Rotate when the log reaches this size.
pub const MAX_BYTES: u64 = 1024 * 1024;

struct FileLogger {
    path: PathBuf,
    file: Mutex<Option<File>>,
    verbose: bool,
}

impl FileLogger {
    fn open(path: &Path) -> Option<File> {
        OpenOptions::new().create(true).append(true).open(path).ok()
    }

    /// Start a new file once the current one is full.
    fn rotate_if_full(&self, file: &mut Option<File>) {
        let full = file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .is_some_and(|m| m.len() >= MAX_BYTES);
        if full {
            *file = None;
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            *file = Self::open(&self.path);
        }
    }
}

impl log::Log for FileLogger {
    /// Dependencies (zbus logs every D-Bus call at info) only from warn up.
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
            && (metadata.level() <= log::Level::Warn || metadata.target().starts_with("pawprint"))
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}\n",
            crate::clock::local(),
            record.level(),
            record.args()
        );
        if self.verbose {
            eprint!("{line}");
        }
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        self.rotate_if_full(&mut file);
        if let Some(f) = file.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

/// Log to `path` (and stderr when `verbose`). Logging never stops the
/// badge: if the file can't be opened, records go to stderr only.
pub fn init(path: &Path, verbose: bool) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = FileLogger::open(path);
    let verbose = verbose || file.is_none();
    let logger = FileLogger {
        path: path.to_path_buf(),
        file: Mutex::new(file),
        verbose,
    };
    if log::set_logger(Box::leak(Box::new(logger))).is_ok() {
        log::set_max_level(if verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_the_cap() {
        let dir = std::env::temp_dir().join(format!("pawprint-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pawprint.log");
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize]).unwrap();
        let logger = FileLogger {
            path: path.clone(),
            file: Mutex::new(FileLogger::open(&path)),
            verbose: false,
        };
        let mut file = logger.file.lock().unwrap();
        logger.rotate_if_full(&mut file);
        file.as_mut().unwrap().write_all(b"fresh\n").unwrap();
        drop(file);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
        assert_eq!(
            std::fs::metadata(path.with_extension("log.1"))
                .unwrap()
                .len(),
            MAX_BYTES
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
