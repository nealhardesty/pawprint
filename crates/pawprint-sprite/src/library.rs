//! The shared building blocks every mascot compiles against: master palette,
//! accents, bases, motion templates, and props (SPEC §12.2).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::grid::parse_grid;
use crate::source::{AccentsFile, MascotFile, MotionFile, PaletteFile, PropFile};
use crate::{Diag, Report, Rgb, is_kebab};

/// A small shared sprite drawn by the `prop:` op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prop {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Option<Rgb>>,
}

#[derive(Debug, Clone, Default)]
pub struct Library {
    pub palette: BTreeMap<String, Rgb>,
    /// In frozen fallback order (SPEC Appendix B).
    pub accents: Vec<(String, Rgb)>,
    pub(crate) bases: BTreeMap<String, MascotFile>,
    pub(crate) motions: BTreeMap<String, MotionFile>,
    pub(crate) props: BTreeMap<String, Prop>,
}

/// Source text for every library file, keyed by file stem where relevant.
/// The bundle embeds it so user mascots can extend bases and use templates
/// and props at runtime (SPEC §12.13).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySources {
    pub palette: String,
    pub accents: String,
    pub bases: Vec<(String, String)>,
    pub motions: Vec<(String, String)>,
    pub props: Vec<(String, String)>,
}

impl LibrarySources {
    /// Read from an `assets/` directory. Missing `bases/`, `motions/`, or
    /// `props/` directories are treated as empty.
    pub fn read(assets: &Path) -> Result<LibrarySources, Report> {
        let mut report = Report::default();
        let read = |rel: &str, report: &mut Report| match std::fs::read_to_string(assets.join(rel))
        {
            Ok(s) => s,
            Err(e) => {
                report.error(rel, format!("cannot read: {e}"));
                String::new()
            }
        };
        let sources = LibrarySources {
            palette: read("palette.toml", &mut report),
            accents: read("accents.toml", &mut report),
            bases: read_dir_toml(&assets.join("bases"), &mut report),
            motions: read_dir_toml(&assets.join("motions"), &mut report),
            props: read_dir_toml(&assets.join("props"), &mut report),
        };
        if report.errors.is_empty() {
            Ok(sources)
        } else {
            Err(report)
        }
    }
}

impl Library {
    pub fn accent(&self, name: &str) -> Option<Rgb> {
        self.accents
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
    }

    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.get(name)
    }

    pub fn motion_names(&self) -> impl Iterator<Item = &str> {
        self.motions.keys().map(String::as_str)
    }

    /// Load from an `assets/` directory.
    pub fn load(assets: &Path) -> Result<Library, Report> {
        Library::from_sources(&LibrarySources::read(assets)?)
    }

    pub fn from_sources(src: &LibrarySources) -> Result<Library, Report> {
        let mut report = Report::default();
        let mut lib = Library::default();

        match toml::from_str::<PaletteFile>(&src.palette) {
            Ok(file) => {
                for (name, hex) in file.colors {
                    match Rgb::parse_hex(&hex) {
                        Some(c) if is_kebab(&name) && name != "clear" => {
                            lib.palette.insert(name, c);
                        }
                        Some(_) => report.error("palette.toml", format!("bad color name {name:?}")),
                        None => {
                            report.error("palette.toml", format!("{name}: {hex:?} is not #rrggbb"))
                        }
                    }
                }
            }
            Err(e) => report.error("palette.toml", e.to_string()),
        }

        match toml::from_str::<AccentsFile>(&src.accents) {
            Ok(file) => {
                for a in file.accents {
                    match Rgb::parse_hex(&a.hex) {
                        Some(c) if lib.accent(&a.name).is_none() => lib.accents.push((a.name, c)),
                        Some(_) => {
                            report.error("accents.toml", format!("duplicate accent {:?}", a.name))
                        }
                        None => report.error(
                            "accents.toml",
                            format!("{}: {:?} is not #rrggbb", a.name, a.hex),
                        ),
                    }
                }
            }
            Err(e) => report.error("accents.toml", e.to_string()),
        }

        for (name, text) in &src.bases {
            let ctx = format!("bases/{name}.toml");
            match toml::from_str::<MascotFile>(text) {
                Ok(file) => {
                    let identity = file.identity_fields();
                    if !identity.is_empty() {
                        report.error(
                            &ctx,
                            format!(
                                "bases have no identity fields; remove {}",
                                identity.join(", ")
                            ),
                        );
                    }
                    for (glyph, color) in &file.palette {
                        if color != "clear" && !lib.palette.contains_key(color) {
                            report.error(
                                &ctx,
                                format!(
                                    "palette {glyph:?}: {color:?} is not a master palette name"
                                ),
                            );
                        }
                    }
                    lib.bases.insert(name.clone(), file);
                }
                Err(e) => report.error(&ctx, e.to_string()),
            }
        }

        for (name, text) in &src.motions {
            let ctx = format!("motions/{name}.toml");
            match toml::from_str::<MotionFile>(text) {
                Ok(m) if &m.name != name => {
                    report.error(&ctx, format!("name {:?} must match the file name", m.name))
                }
                Ok(m) if m.frames.iter().any(|f| f.grid.is_some()) => {
                    report.error(&ctx, "motion templates use layer ops only, not grids");
                }
                Ok(m) if m.frames.is_empty() => {
                    report.error(&ctx, "a motion needs at least one frame")
                }
                Ok(m) => {
                    lib.motions.insert(name.clone(), m);
                }
                Err(e) => report.error(&ctx, e.to_string()),
            }
        }

        for (name, text) in &src.props {
            let ctx = format!("props/{name}.toml");
            match toml::from_str::<PropFile>(text) {
                Ok(p) if &p.name != name => {
                    report.error(&ctx, format!("name {:?} must match the file name", p.name))
                }
                Ok(p) => match compile_prop(&p, &lib.palette) {
                    Ok(prop) => {
                        lib.props.insert(name.clone(), prop);
                    }
                    Err(msg) => report.error(&ctx, msg),
                },
                Err(e) => report.error(&ctx, e.to_string()),
            }
        }

        if report.errors.is_empty() {
            Ok(lib)
        } else {
            Err(report)
        }
    }
}

fn compile_prop(p: &PropFile, palette: &BTreeMap<String, Rgb>) -> Result<Prop, String> {
    let grid = parse_grid(&p.grid)?;
    let mut glyphs = BTreeMap::new();
    for (glyph, color) in &p.palette {
        let c = single_char(glyph)?;
        let rgb = match color.as_str() {
            "clear" => None,
            name => Some(
                *palette
                    .get(name)
                    .ok_or_else(|| format!("{name:?} is not a master palette color"))?,
            ),
        };
        glyphs.insert(c, rgb);
    }
    let pixels = grid
        .glyphs
        .iter()
        .map(|g| {
            glyphs
                .get(g)
                .copied()
                .ok_or_else(|| format!("glyph {g:?} is not in the palette"))
        })
        .collect::<Result<_, _>>()?;
    Ok(Prop {
        width: grid.width,
        height: grid.height,
        pixels,
    })
}

pub(crate) fn single_char(s: &str) -> Result<char, String> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_whitespace() => Ok(c),
        _ => Err(format!(
            "palette key {s:?} must be exactly one non-space character"
        )),
    }
}

/// Read every `*.toml` in a directory as (file stem, contents), sorted.
fn read_dir_toml(dir: &Path, report: &mut Report) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            match std::fs::read_to_string(&path) {
                Ok(text) => out.push((stem, text)),
                Err(e) => report.errors.push(Diag::new(
                    path.display().to_string(),
                    format!("cannot read: {e}"),
                )),
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const PALETTE: &str = r##"
[colors]
ink = "#1b1b1f"
white = "#f7f5ef"
cream = "#f2efe6"
brown-1 = "#d9b48a"
brown-2 = "#a8784d"
brown-3 = "#74502f"
grey-2 = "#9a9893"
fur-black = "#2b2833"
amber = "#f0a830"
orange-2 = "#e8912d"
red-1 = "#e86a5f"
"##;

    pub const ACCENTS: &str = r##"
accents = [
  { name = "violet", hex = "#7b5ea7" },
  { name = "pine", hex = "#2f6b4f" },
]
"##;

    pub fn lib_with(
        bases: &[(&str, &str)],
        motions: &[(&str, &str)],
        props: &[(&str, &str)],
    ) -> Library {
        let own = |v: &[(&str, &str)]| {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        Library::from_sources(&LibrarySources {
            palette: PALETTE.into(),
            accents: ACCENTS.into(),
            bases: own(bases),
            motions: own(motions),
            props: own(props),
        })
        .unwrap()
    }

    #[test]
    fn loads_palette_and_accents_in_order() {
        let lib = lib_with(&[], &[], &[]);
        assert_eq!(lib.palette["ink"], Rgb(0x1b, 0x1b, 0x1f));
        assert_eq!(lib.accents[0].0, "violet");
        assert_eq!(lib.accent("pine"), Some(Rgb(0x2f, 0x6b, 0x4f)));
        assert_eq!(lib.accent("nope"), None);
    }

    #[test]
    fn rejects_bad_library_files() {
        let bad_motion = ("blink", "name = \"wink\"\nframes = [{ ops = [] }]");
        let bad_prop = (
            "heart",
            "name = \"heart\"\npalette = { r = \"crimson\" }\ngrid = \"r\"",
        );
        let bad_base = ("cat", "id = \"cat\"");
        let err = Library::from_sources(&LibrarySources {
            palette: PALETTE.into(),
            accents: ACCENTS.into(),
            bases: vec![(bad_base.0.into(), bad_base.1.into())],
            motions: vec![(bad_motion.0.into(), bad_motion.1.into())],
            props: vec![(bad_prop.0.into(), bad_prop.1.into())],
        })
        .unwrap_err();
        let text: Vec<String> = err.errors.iter().map(ToString::to_string).collect();
        assert!(
            text.iter()
                .any(|t| t.contains("bases/cat.toml") && t.contains("identity")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.contains("motions/blink.toml") && t.contains("match the file name")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.contains("props/heart.toml") && t.contains("crimson")),
            "{text:?}"
        );
    }

    #[test]
    fn compiles_props() {
        let lib = lib_with(
            &[],
            &[],
            &[(
                "heart",
                "name = \"heart\"\npalette = { \".\" = \"clear\", r = \"red-1\" }\ngrid = \"\"\"\nr.r\n.r.\n\"\"\"",
            )],
        );
        let p = &lib.props["heart"];
        assert_eq!((p.width, p.height), (3, 2));
        assert_eq!(p.pixels[1], None);
        assert_eq!(p.pixels[0], Some(Rgb(0xe8, 0x6a, 0x5f)));
    }
}
