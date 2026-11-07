//! `state.toml`: the dragged position and the fallback-mascot cache
//! (SPEC §8.4). Owned by PAWPRINT and rewritten atomically.

use std::collections::BTreeMap;
use std::io::{self, Write};
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
    /// Parse a state file. Unknown keys and invalid entries are skipped
    /// one by one, each with a warning, and the rest is kept. `Err` means
    /// the file can't be used at all: not TOML, or no supported `version`.
    pub fn parse(text: &str) -> Result<(State, Vec<String>), String> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        match table.get("version").and_then(toml::Value::as_integer) {
            Some(v) if v == i64::from(STATE_VERSION) => {}
            Some(v) => return Err(format!("unsupported state version {v}")),
            None => return Err("no `version`".into()),
        }
        let mut state = State::default();
        let mut warnings = Vec::new();
        for (key, value) in table {
            match key.as_str() {
                "version" => {}
                "position" => match value.try_into::<Position>() {
                    Ok(p) => state.position = Some(p),
                    Err(e) => warnings.push(format!("[position] ignored: {e}")),
                },
                "assignments" => match value {
                    toml::Value::Table(t) => {
                        for (host, id) in t {
                            match id.as_str() {
                                Some(id) if pawprint_sprite::is_valid_id(id) => {
                                    state.assignments.insert(host, id.to_string());
                                }
                                _ => warnings.push(format!(
                                    "assignments.{host} = {id} ignored: not a mascot id"
                                )),
                            }
                        }
                    }
                    _ => warnings.push("`assignments` ignored: not a table".into()),
                },
                _ => warnings.push(format!("unknown key `{key}` ignored")),
            }
        }
        Ok((state, warnings))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("state always serializes")
    }

    /// Read the state file. A missing file is an empty state; invalid
    /// entries are dropped with a warning; an unreadable or unparseable file
    /// is an empty state plus a warning (and is kept as `state.toml.bad` at
    /// the next save), so a bad state file never stops the badge.
    pub fn load(path: &Path) -> (State, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match State::parse(&text) {
                Ok((s, w)) if w.is_empty() => (s, None),
                Ok((s, w)) => (s, Some(format!("{}: {}", path.display(), w.join("; ")))),
                Err(e) => (
                    State::default(),
                    Some(format!(
                        "{}: {e}; starting fresh (the next save keeps it as {})",
                        path.display(),
                        path.with_extension("toml.bad").display()
                    )),
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (State::default(), None),
            Err(e) => (
                State::default(),
                Some(format!("{}: {e}; starting fresh", path.display())),
            ),
        }
    }

    /// Write to a temporary file next to `path`, flush it to disk, then
    /// rename over it, so a crash never leaves a half-written state file. An
    /// existing file that can't be parsed at all is first copied to
    /// `state.toml.bad`, so overwriting it loses nothing.
    pub fn save_atomic(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if let Ok(old) = std::fs::read_to_string(path)
            && State::parse(&old).is_err()
        {
            std::fs::write(path.with_extension("toml.bad"), old)?;
        }
        let tmp = path.with_extension("toml.tmp");
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(self.to_toml().as_bytes())?;
        file.sync_all()?;
        drop(file);
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
        let (s, warnings) = State::parse(text).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            s.position,
            Some(Position {
                anchor: Anchor::TopRight,
                offset: [16, 16]
            })
        );
        assert_eq!(s.assignments["buildbox"], "capybara");
        assert_eq!(State::parse(&s.to_toml()).unwrap(), (s, vec![]));
    }

    #[test]
    fn minimal_and_invalid() {
        assert_eq!(
            State::parse("version = 1").unwrap(),
            (State::default(), vec![])
        );
        assert!(State::parse("version = 2").is_err());
        assert!(State::parse("position = 3").is_err(), "no version");
        assert!(State::parse("version = 1\n[x").is_err());
        assert!(!State::default().to_toml().contains("position"));
    }

    #[test]
    fn bad_entries_are_skipped_one_by_one() {
        let text = "version = 1\nshiny = true\n\n[position]\nanchor = \"middle\"\noffset = [1, 2]\n\n[assignments]\nbuildbox = \"capybara\"\nweird = 3\nupper = \"Not An Id\"\n";
        let (s, warnings) = State::parse(text).unwrap();
        assert_eq!(s.position, None);
        assert_eq!(
            s.assignments.into_iter().collect::<Vec<_>>(),
            vec![("buildbox".to_string(), "capybara".to_string())]
        );
        assert_eq!(warnings.len(), 4, "{warnings:#?}");

        let (s, warnings) = State::parse(
            "version = 1\nassignments = 3\n[position]\nanchor = \"top\"\noffset = [0, 9]",
        )
        .unwrap();
        assert_eq!(s.position.unwrap().offset, [0, 9]);
        assert_eq!(warnings.len(), 1, "{warnings:#?}");
    }

    #[test]
    fn an_unparseable_file_is_kept_before_it_is_overwritten() {
        let dir = std::env::temp_dir().join(format!("pawprint-state-bad-{}", std::process::id()));
        let path = dir.join("state.toml");
        let bad = path.with_extension("toml.bad");
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(&path, "version = 1\n[assignments]\nx = 3\n").unwrap();
        State::default().save_atomic(&path).unwrap();
        assert!(
            !bad.exists(),
            "a parseable file with bad entries is not kept"
        );

        std::fs::write(&path, "garbage =").unwrap();
        State::default().save_atomic(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "garbage =");
        assert_eq!(State::load(&path), (State::default(), None));
        std::fs::remove_dir_all(&dir).unwrap();
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
