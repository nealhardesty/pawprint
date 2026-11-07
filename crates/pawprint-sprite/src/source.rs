//! Serde shapes of the TOML source files (SPEC §12.3, §12.4, §12.7).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// When an action may play, by the local clock. Night is 20:00–06:00.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    #[default]
    Any,
    Day,
    Night,
}

impl When {
    /// Does this action's `when` allow it at the given local hour (0–23)?
    pub fn matches(self, hour: u8) -> bool {
        let night = !(6..20).contains(&hour);
        match self {
            When::Any => true,
            When::Day => !night,
            When::Night => night,
        }
    }
}

/// A mascot (`assets/mascots/<id>/mascot.toml`) or base
/// (`assets/bases/<name>.toml`) file. Bases use the same shape without the
/// identity fields. PNG-mode mascots set `sprite` and use frame indices.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MascotFile {
    pub id: Option<String>,
    pub name: Option<String>,
    pub category: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub accent: Option<String>,
    pub fallback: Option<bool>,
    pub nocturnal: Option<bool>,
    pub extends: Option<String>,
    pub sprite: Option<String>,
    /// Shifts every prop that a motion template draws (`doze`'s zzz,
    /// `content`'s heart, ...) by `[dx, dy]`, for mascots whose top-right
    /// corner is busy. Props in the mascot's own actions are not moved.
    pub prop_offset: Option<[i32; 2]>,
    #[serde(default)]
    pub palette: BTreeMap<String, String>,
    /// Ordered: layer order is drawing order.
    #[serde(default)]
    pub layers: toml::Table,
    /// A list of layer names (text mode) or a frame index (PNG mode).
    pub rest: Option<toml::Value>,
    #[serde(default)]
    pub actions: Vec<ActionSrc>,
    #[serde(default)]
    pub frames: BTreeMap<String, FrameGridSrc>,
}

impl MascotFile {
    /// Names of identity fields that are set, for rejecting them in bases.
    pub fn identity_fields(&self) -> Vec<&'static str> {
        let set = [
            ("id", self.id.is_some()),
            ("name", self.name.is_some()),
            ("category", self.category.is_some()),
            ("aliases", !self.aliases.is_empty()),
            ("accent", self.accent.is_some()),
            ("fallback", self.fallback.is_some()),
            ("nocturnal", self.nocturnal.is_some()),
            ("extends", self.extends.is_some()),
            ("sprite", self.sprite.is_some()),
        ];
        set.iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| *name)
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSrc {
    pub at: Option<[i32; 2]>,
    /// Another slot whose offsets this layer's slot also takes, so `head+1,0`
    /// carries the eyes, ears, and nose with it.
    pub follows: Option<String>,
    pub grid: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameGridSrc {
    pub grid: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSrc {
    pub name: Option<String>,
    #[serde(rename = "use")]
    pub template: Option<String>,
    pub weight: Option<u32>,
    pub when: Option<When>,
    pub frames: Option<Vec<FrameSrc>>,
    /// PNG mode only: duration of every frame in the action.
    pub duration_ms: Option<u32>,
}

/// A step is a table in text mode and a frame index in PNG mode.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FrameSrc {
    Index(u32),
    Step(StepSrc),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSrc {
    #[serde(default)]
    pub ops: Vec<String>,
    pub grid: Option<String>,
    pub ms: Option<u32>,
}

/// `assets/motions/<name>.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionFile {
    pub name: String,
    /// One line for humans; not used by the compiler.
    #[serde(default)]
    #[allow(dead_code)]
    pub summary: String,
    #[serde(default)]
    pub needs: Vec<String>,
    pub weight: Option<u32>,
    pub when: Option<When>,
    pub frames: Vec<StepSrc>,
}

/// `assets/props/<name>.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropFile {
    pub name: String,
    pub palette: BTreeMap<String, String>,
    pub grid: String,
}

/// `assets/palette.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteFile {
    pub colors: BTreeMap<String, String>,
}

/// `assets/accents.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccentsFile {
    pub accents: Vec<NamedHex>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedHex {
    pub name: String,
    pub hex: String,
}

/// `assets/catalog.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogFile {
    pub version: u32,
    pub required: Vec<String>,
    pub minimum: u32,
}

/// `assets/font/font.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFile {
    pub glyphs: BTreeMap<String, String>,
}

/// `assets/crate/crate.toml`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrateFile {
    pub palette: BTreeMap<String, String>,
    pub layers: BTreeMap<String, LayerSrc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_matches_local_hour() {
        let cases = [
            (0, false, true),
            (5, false, true),
            (6, true, false),
            (12, true, false),
            (19, true, false),
            (20, false, true),
            (23, false, true),
        ];
        for (hour, day, night) in cases {
            assert!(When::Any.matches(hour));
            assert_eq!(When::Day.matches(hour), day, "day at {hour}");
            assert_eq!(When::Night.matches(hour), night, "night at {hour}");
        }
    }
}
