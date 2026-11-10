//! The embedded asset bundle and the mascot catalog (SPEC §12.12–12.13),
//! plus user mascots from the config `mascots/` folder (SPEC §12.7).

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pawprint_sprite::{Bundle, Library, Mascot, Origin, compile_dir};

use crate::matching::AliasTable;

static BUNDLE_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundle.bin"));

/// Every bundled asset, compiled and validated by `build.rs`.
pub fn bundle() -> &'static Bundle {
    static BUNDLE: OnceLock<Bundle> = OnceLock::new();
    BUNDLE.get_or_init(|| Bundle::from_bytes(BUNDLE_BYTES).expect("build.rs embeds a valid bundle"))
}

/// The bundled bases, motion templates, and props that text-format user
/// mascots compile against (SPEC §12.13).
pub fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        bundle()
            .library()
            .expect("build.rs validated the embedded library")
    })
}

/// Where a catalog entry comes from (the SOURCE column of `pawprint list`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Bundled,
    /// A user mascot with a new id.
    User,
    /// A user mascot that replaces the bundled one with the same id.
    Override,
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Provenance::Bundled => "bundled",
            Provenance::User => "user",
            Provenance::Override => "override",
        })
    }
}

/// The mascots a badge can show, with their alias table.
#[derive(Debug, Clone)]
pub struct Catalog {
    mascots: Vec<Mascot>,
    provenance: Vec<Provenance>,
    aliases: AliasTable,
    warnings: Vec<String>,
}

impl Catalog {
    /// A catalog of bundled mascots only.
    pub fn new(mascots: Vec<Mascot>) -> Catalog {
        Catalog::with_user(mascots, Vec::new())
    }

    /// Bundled plus user mascots (SPEC §12.11 rule 8). A user mascot with a
    /// bundled id replaces it in place; a new id joins the end. On an alias
    /// collision the user mascot wins, and [`Catalog::warnings`] says so.
    pub fn with_user(bundled: Vec<Mascot>, user: Vec<Mascot>) -> Catalog {
        let mut provenance = vec![Provenance::Bundled; bundled.len()];
        let mut mascots = bundled;
        for m in user {
            match mascots.iter().position(|b| b.id == m.id) {
                Some(i) => {
                    mascots[i] = m;
                    provenance[i] = Provenance::Override;
                }
                None => {
                    mascots.push(m);
                    provenance.push(Provenance::User);
                }
            }
        }
        // Bundled aliases go in first, so user aliases take them over.
        let mut aliases = AliasTable::default();
        let mut warnings = Vec::new();
        let (bundled, user): (Vec<_>, Vec<_>) = mascots
            .iter()
            .zip(&provenance)
            .partition(|(_, p)| **p == Provenance::Bundled);
        for (m, p) in bundled.into_iter().chain(user) {
            for (alias, other) in aliases.insert(&m.id, &m.aliases) {
                if *p != Provenance::Bundled {
                    warnings.push(format!(
                        "user mascot {id}: alias {alias:?} also names {other}; {id} wins",
                        id = m.id
                    ));
                }
            }
        }
        Catalog {
            mascots,
            provenance,
            aliases,
            warnings,
        }
    }

    /// The bundled catalog plus every valid mascot in `user_dir` (the config
    /// `mascots/` folder). Invalid user mascots are skipped with a warning;
    /// they never stop the badge.
    pub fn load(user_dir: &Path) -> Catalog {
        let (user, mut warnings) = load_user_mascots(user_dir, library());
        let mut catalog = Catalog::with_user(bundle().mascots.clone(), user);
        warnings.append(&mut catalog.warnings);
        catalog.warnings = warnings;
        catalog
    }

    /// The bundled catalog.
    pub fn bundled() -> &'static Catalog {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| Catalog::new(bundle().mascots.clone()))
    }

    pub fn get(&self, id: &str) -> Option<&Mascot> {
        self.mascots.iter().find(|m| m.id == id)
    }

    pub fn mascots(&self) -> &[Mascot] {
        &self.mascots
    }

    /// Every mascot with where it came from, in catalog order.
    pub fn entries(&self) -> impl Iterator<Item = (&Mascot, Provenance)> {
        self.mascots.iter().zip(self.provenance.iter().copied())
    }

    pub fn aliases(&self) -> &AliasTable {
        &self.aliases
    }

    /// Problems met while adding user mascots, for the log or stderr.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Ids eligible for the deterministic fallback (manifest `fallback`
    /// not false).
    pub fn eligible(&self) -> impl Iterator<Item = &str> {
        self.mascots
            .iter()
            .filter(|m| m.fallback)
            .map(|m| m.id.as_str())
    }
}

/// Compile every `<dir>/<id>/` (in name order) with the user rules. Returns
/// the valid mascots, plus a warning for each skipped mascot and each
/// finding. A missing folder is normal: most people have none.
pub fn load_user_mascots(dir: &Path, lib: &Library) -> (Vec<Mascot>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            warnings.push(format!("cannot read {}: {e}", dir.display()));
            Vec::new()
        }
    };
    dirs.sort();
    let mut mascots = Vec::new();
    for path in dirs {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let compiled = compile_dir(&path, lib, Origin::User);
        let report = compiled.report;
        match compiled.mascot {
            Some(m) => {
                let found = report.warnings.iter();
                warnings.extend(found.map(|w| format!("user mascot {name}: {w}")));
                mascots.push(m);
            }
            None => {
                let errors: Vec<String> = report.errors.iter().map(|e| e.to_string()).collect();
                warnings.push(format!(
                    "user mascot {name} skipped ({}): {}",
                    path.display(),
                    errors.join("; ")
                ));
            }
        }
    }
    (mascots, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_has_the_owl() {
        let owl = Catalog::bundled().get("owl").expect("owl is bundled");
        assert_eq!(owl.accent, "violet");
        assert!(owl.actions.iter().any(|a| a.name == "blink"));
        assert_eq!(Catalog::bundled().aliases().lookup("hoot"), Some("owl"));
        assert!(Catalog::bundled().eligible().any(|id| id == "owl"));
        assert!(
            Catalog::bundled()
                .entries()
                .all(|(_, p)| p == Provenance::Bundled)
        );
        assert!(Catalog::bundled().warnings().is_empty());
    }

    const MY_CAT: &str = r##"
id = "my-cat"
name = "My Cat"
category = "cats-dogs"
aliases = ["hoot", "whiskers"]
accent = "#336699"
extends = "cat-sit"

[palette]
"f" = "#c08040"

[[actions]]
use = "blink"

[[actions]]
name = "wave"
frames = [{ ops = ["head+1,0", "prop:heart@22,1"] }]
"##;

    #[test]
    fn user_text_mascots_can_use_the_embedded_library() {
        let c = pawprint_sprite::compile(MY_CAT, Some("my-cat"), library(), Origin::User, &|n| {
            Err(n.to_string())
        });
        assert!(c.report.errors.is_empty(), "{:?}", c.report.errors);
        let m = c.mascot.unwrap();
        assert_eq!(m.base.as_deref(), Some("cat-sit"));
        assert!(m.colors.contains(&pawprint_sprite::Rgb(0xc0, 0x80, 0x40)));
    }

    fn renamed(id: &str, name: &str) -> Mascot {
        let mut m = bundle().mascot(id).unwrap().clone();
        m.name = name.into();
        m
    }

    #[test]
    fn user_mascots_override_add_and_win_alias_collisions() {
        let mut new = renamed("bear", "Mine");
        new.id = "my-bear".into();
        new.aliases = vec!["hoot".into(), "teddy".into()];
        let c = Catalog::with_user(
            bundle().mascots.clone(),
            vec![renamed("owl", "My Owl"), new],
        );
        let n = bundle().mascots.len();
        assert_eq!(c.mascots().len(), n + 1);
        let source = |id: &str| c.entries().find(|(m, _)| m.id == id).unwrap().1;
        assert_eq!(source("owl"), Provenance::Override);
        assert_eq!(source("my-bear"), Provenance::User);
        assert_eq!(source("moose"), Provenance::Bundled);
        assert_eq!(c.get("owl").unwrap().name, "My Owl");
        assert_eq!(c.aliases().lookup("owl"), Some("owl"));
        assert_eq!(c.aliases().lookup("teddy"), Some("my-bear"));
        assert_eq!(c.aliases().lookup("hoot"), Some("my-bear"), "user wins");
        assert_eq!(
            c.warnings(),
            ["user mascot my-bear: alias \"hoot\" also names owl; my-bear wins"]
        );
        assert!(c.eligible().any(|id| id == "my-bear"));
    }

    /// A fresh, empty directory under the system temp dir.
    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pawprint-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, file: &str, bytes: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(file), bytes).unwrap();
    }

    #[test]
    fn load_reads_text_and_png_mascots_and_skips_bad_ones() {
        let root = temp_dir("user-mascots");
        write(&root.join("my-cat"), "mascot.toml", MY_CAT.as_bytes());
        let (toml, png) = pawprint_sprite::export_png(bundle().mascot("owl").unwrap());
        write(&root.join("owl"), "mascot.toml", toml.as_bytes());
        write(&root.join("owl"), "sprite.png", &png.encode_png());
        write(&root.join("broken"), "mascot.toml", b"id = \"wrong\"");
        write(&root, "notes.txt", b"not a mascot");

        let c = Catalog::load(&root);
        let source = |id: &str| c.entries().find(|(m, _)| m.id == id).map(|e| e.1);
        assert_eq!(source("my-cat"), Some(Provenance::User));
        assert_eq!(source("owl"), Some(Provenance::Override));
        assert_eq!(source("broken"), None);
        assert_eq!(c.mascots().len(), bundle().mascots.len() + 1);
        assert!(
            c.warnings()
                .iter()
                .any(|w| w.starts_with("user mascot broken skipped"))
        );
        assert!(
            c.warnings()
                .iter()
                .any(|w| w.contains("\"hoot\" also names owl"))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_without_a_mascots_folder_is_the_bundled_catalog() {
        let c = Catalog::load(Path::new("/nonexistent/pawprint/mascots"));
        assert_eq!(c.mascots().len(), bundle().mascots.len());
        assert!(c.warnings().is_empty());
    }

    #[test]
    fn bundle_has_font_and_crate() {
        let b = bundle();
        assert_eq!(b.font.glyphs.len(), 95);
        assert_eq!(b.crate_art.front.height, 12);
        assert_eq!(b.minimum, 214);
    }
}
