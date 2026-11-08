//! The bundled catalog: every asset compiled and validated together, plus a
//! compact binary form for embedding (SPEC §12.11–12.13).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::source::CatalogFile;
use crate::{
    CrateArt, Diag, Font, Library, LibrarySources, Mascot, Origin, Report, Rgb, compile_base,
    compile_dir, squash,
};

/// Everything the binary embeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    /// Accent palette in frozen fallback order.
    pub accents: Vec<(String, Rgb)>,
    pub font: Font,
    pub crate_art: CrateArt,
    /// Sorted by id.
    pub mascots: Vec<Mascot>,
    /// Catalog gates, enforced only when packaging.
    pub required: Vec<String>,
    pub minimum: u32,
    /// Palette, accents, bases, motion templates, and props as source text,
    /// so text-format user mascots can use them at runtime.
    pub library: LibrarySources,
}

impl Bundle {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("serializing to memory cannot fail")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Bundle, String> {
        postcard::from_bytes(bytes).map_err(|e| e.to_string())
    }

    pub fn mascot(&self, id: &str) -> Option<&Mascot> {
        self.mascots.iter().find(|m| m.id == id)
    }

    /// The building blocks user mascots compile against. The build already
    /// validated them, so this cannot fail for a bundle from `build_bundle`.
    pub fn library(&self) -> Result<Library, Report> {
        Library::from_sources(&self.library)
    }
}

/// The result of compiling `assets/`. `bundle` is `Some` only when there
/// are no errors.
#[derive(Debug)]
pub struct BundleReport {
    pub bundle: Option<Bundle>,
    pub report: Report,
}

/// Compile and validate the whole `assets/` directory (rules 1–8 of SPEC
/// §12.11). Warnings in bundled mascots are errors. The catalog gates
/// (rule 9) are separate: see [`check_gates`].
pub fn build_bundle(assets: &Path) -> BundleReport {
    let mut report = Report::default();
    let loaded = LibrarySources::read(assets)
        .and_then(|src| Library::from_sources(&src).map(|lib| (src, lib)));
    let (sources, lib) = match loaded {
        Ok(pair) => pair,
        Err(r) => {
            report.absorb("assets", r);
            return BundleReport {
                bundle: None,
                report,
            };
        }
    };
    let read = |rel: &str, report: &mut Report| {
        std::fs::read_to_string(assets.join(rel))
            .map_err(|e| report.error(rel, format!("cannot read: {e}")))
            .ok()
    };

    let catalog = read("catalog.toml", &mut report).and_then(|s| {
        toml::from_str::<CatalogFile>(&s)
            .map_err(|e| report.error("catalog.toml", e.to_string()))
            .ok()
    });
    if let Some(c) = &catalog
        && c.version != 1
    {
        report.error("catalog.toml", format!("unsupported version {}", c.version));
    }
    let font = read("font/font.toml", &mut report).and_then(|s| {
        Font::compile(&s)
            .map_err(|r| report.absorb("font/font.toml", r))
            .ok()
    });
    let crate_art = read("crate/crate.toml", &mut report).and_then(|s| {
        CrateArt::compile(&s, &lib)
            .map_err(|r| report.absorb("crate/crate.toml", r))
            .ok()
    });
    if lib.accents.len() < 16 {
        report.error(
            "accents.toml",
            format!("needs the 16 fallback accents (has {})", lib.accents.len()),
        );
    }

    // Every base must stand on its own as a generic animal, with every
    // template whose needs it meets, even before any mascot extends it.
    for (name, _) in &sources.bases {
        let mut compiled = compile_base(&lib, name);
        compiled.report.errors.append(&mut compiled.report.warnings);
        report.absorb(&format!("bases/{name}.toml"), compiled.report);
    }

    let mut mascots = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(assets.join("mascots"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    for dir in dirs {
        let ctx = format!(
            "mascots/{}",
            dir.file_name().unwrap_or_default().to_string_lossy()
        );
        let mut compiled = compile_dir(&dir, &lib, Origin::Bundled);
        // Rule 4: a warning in a bundled mascot fails the build.
        compiled.report.errors.append(&mut compiled.report.warnings);
        report.absorb(&ctx, compiled.report);
        mascots.extend(compiled.mascot);
    }
    check_aliases(&mascots, &mut report);
    if report.errors.is_empty() && !mascots.iter().any(|m| m.fallback) {
        report.error(
            "mascots",
            "the catalog needs at least one mascot eligible for the fallback",
        );
    }

    let bundle = match (catalog, font, crate_art) {
        (Some(catalog), Some(font), Some(crate_art)) if report.errors.is_empty() => Some(Bundle {
            accents: lib.accents,
            font,
            crate_art,
            mascots,
            required: catalog.required,
            minimum: catalog.minimum,
            library: sources,
        }),
        _ => None,
    };
    BundleReport { bundle, report }
}

/// Rule 8: squashed aliases (and ids) are unique across the catalog.
fn check_aliases(mascots: &[Mascot], report: &mut Report) {
    let mut owner: BTreeMap<String, &str> = BTreeMap::new();
    for m in mascots {
        let mut names: Vec<String> = std::iter::once(&m.id)
            .chain(&m.aliases)
            .map(|a| squash(a))
            .collect();
        names.sort();
        names.dedup();
        for name in names {
            if let Some(other) = owner.insert(name.clone(), &m.id) {
                report.error(
                    format!("mascots/{}", m.id),
                    format!("alias {name:?} is already used by {other:?}"),
                );
            }
        }
    }
}

/// Rule 9, the catalog gates. Only `make package` enforces these; `make test`
/// and `make check-assets` report them for information, because the catalog
/// stays small until the art milestones land.
///
/// - at least `minimum` mascots;
/// - every `required` id present;
/// - every required (showcase) mascot has at least 2 template actions, a
///   signature action (non-template, weight above 1), and a rare action
///   (weight 1).
pub fn check_gates(bundle: &Bundle) -> Vec<Diag> {
    let mut out = Vec::new();
    if bundle.mascots.len() < bundle.minimum as usize {
        out.push(Diag::new(
            "catalog",
            format!(
                "has {} mascots; at least {} are required",
                bundle.mascots.len(),
                bundle.minimum
            ),
        ));
    }
    for id in &bundle.required {
        let Some(m) = bundle.mascot(id) else {
            out.push(Diag::new(
                "catalog",
                format!("required mascot {id:?} is missing"),
            ));
            continue;
        };
        let templates = m.actions.iter().filter(|a| a.template.is_some()).count();
        let signature = m
            .actions
            .iter()
            .any(|a| a.template.is_none() && a.weight > 1);
        let rare = m.actions.iter().any(|a| a.weight == 1);
        if templates < 2 || !signature || !rare {
            out.push(Diag::new(
                format!("mascots/{id}"),
                format!("showcase mascots need 2+ template actions ({templates}), a signature action ({signature}), and a rare action ({rare})"),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;
    use crate::source::When;

    fn mascot(id: &str, aliases: &[&str], actions: Vec<Action>) -> Mascot {
        Mascot {
            id: id.into(),
            name: id.into(),
            category: "birds".into(),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            accent: "violet".into(),
            fallback: true,
            nocturnal: false,
            base: None,
            colors: vec![],
            frames: vec![vec![0; 1024]],
            actions,
        }
    }

    fn action(template: Option<&str>, weight: u32) -> Action {
        Action {
            name: "a".into(),
            weight,
            when: When::Any,
            template: template.map(Into::into),
            frames: vec![],
        }
    }

    fn bundle(mascots: Vec<Mascot>, required: &[&str], minimum: u32) -> Bundle {
        Bundle {
            accents: vec![],
            font: Font { glyphs: vec![] },
            crate_art: CrateArt {
                colors: vec![],
                back: empty(),
                front: empty(),
                plate_left: empty(),
                plate_mid: empty(),
                plate_right: empty(),
            },
            mascots,
            required: required.iter().map(|r| r.to_string()).collect(),
            minimum,
            library: LibrarySources::default(),
        }
    }

    fn empty() -> crate::IndexedLayer {
        crate::IndexedLayer {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            pixels: vec![],
        }
    }

    #[test]
    fn alias_collisions_are_errors() {
        let mut report = Report::default();
        check_aliases(
            &[
                mascot("polar-bear", &[], vec![]),
                mascot("bear", &["polar_bear", "bear"], vec![]),
            ],
            &mut report,
        );
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(report.errors[0].message.contains("\"polarbear\""));
    }

    #[test]
    fn gates() {
        let full = vec![
            action(Some("blink"), 6),
            action(Some("bob"), 5),
            action(None, 3),
            action(None, 1),
        ];
        let ok = bundle(
            vec![mascot("owl", &[], full.clone()), mascot("moose", &[], full)],
            &["owl"],
            2,
        );
        assert!(check_gates(&ok).is_empty());

        let small = bundle(
            vec![mascot("owl", &[], vec![action(None, 3)])],
            &["owl", "moose"],
            100,
        );
        let diags: Vec<String> = check_gates(&small)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert!(diags[0].contains("at least 100"));
        assert!(diags[1].contains("mascots/owl"));
        assert!(diags[2].contains("\"moose\" is missing"));
    }

    #[test]
    fn bytes_round_trip() {
        let b = bundle(
            vec![mascot("owl", &["hoot"], vec![action(None, 3)])],
            &["owl"],
            1,
        );
        assert_eq!(Bundle::from_bytes(&b.to_bytes()).unwrap(), b);
        assert!(Bundle::from_bytes(b"\xff").is_err());
    }
}
