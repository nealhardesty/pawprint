//! `state.toml`: the dragged position and the fallback-mascot cache
//! (SPEC §8.4). Owned by PAWPRINT and rewritten atomically.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::layout::Position;

pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    /// Present only after a drag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    /// Rule-5 cache: short hostname → mascot id.
    #[serde(default)]
    pub assignments: BTreeMap<String, String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            version: STATE_VERSION,
            position: None,
            assignments: BTreeMap::new(),
        }
    }
}

impl State {
    pub fn parse(text: &str) -> Result<State, String> {
        let state: State = toml::from_str(text).map_err(|e| e.to_string())?;
        if state.version != STATE_VERSION {
            return Err(format!("unsupported state version {}", state.version));
        }
        Ok(state)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("state always serializes")
    }

    /// Read the state file. A missing file is an empty state; an unreadable
    /// or invalid one is an empty state plus a warning, so a bad state file
    /// never stops the badge.
    pub fn load(path: &Path) -> (State, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match State::parse(&text) {
                Ok(s) => (s, None),
                Err(e) => (
                    State::default(),
                    Some(format!("{}: {e}; starting fresh", path.display())),
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (State::default(), None),
            Err(e) => (
                State::default(),
                Some(format!("{}: {e}; starting fresh", path.display())),
            ),
        }
    }

    /// Write to a temporary file next to `path`, then rename over it, so a
    /// crash never leaves a half-written state file.
    pub fn save_atomic(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml())?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Anchor;

    #[test]
    fn parses_spec_example() {
        let text = "version = 1\n\n[position]\nanchor = \"top-right\"\noffset = [16, 16]\n\n[assignments]\nbuildbox = \"capybara\"\n";
        let s = State::parse(text).unwrap();
        assert_eq!(
            s.position,
            Some(Position {
                anchor: Anchor::TopRight,
                offset: [16, 16]
            })
        );
        assert_eq!(s.assignments["buildbox"], "capybara");
        assert_eq!(State::parse(&s.to_toml()).unwrap(), s);
    }

    #[test]
    fn minimal_and_invalid() {
        assert_eq!(State::parse("version = 1").unwrap(), State::default());
        assert!(State::parse("version = 2").is_err());
        assert!(State::parse("position = 3").is_err());
        assert!(!State::default().to_toml().contains("position"));
    }

    #[test]
    fn atomic_save_and_load() {
        let dir = std::env::temp_dir().join(format!("pawprint-state-test-{}", std::process::id()));
        let path = dir.join("nested").join("state.toml");
        let (empty, warning) = State::load(&path);
        assert_eq!(
            (empty, warning),
            (State::default(), None),
            "missing file is fine"
        );

        let mut s = State::default();
        s.assignments.insert("buildbox".into(), "owl".into());
        s.save_atomic(&path).unwrap();
        s.position = Some(Position {
            anchor: Anchor::BottomLeft,
            offset: [3, 4],
        });
        s.save_atomic(&path).unwrap(); // overwrite an existing file
        assert_eq!(State::load(&path), (s, None));
        assert!(!path.with_extension("toml.tmp").exists());

        std::fs::write(&path, "garbage =").unwrap();
        let (fresh, warning) = State::load(&path);
        assert_eq!(fresh, State::default());
        assert!(warning.unwrap().contains("starting fresh"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
