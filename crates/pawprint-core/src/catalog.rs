//! The embedded asset bundle and the mascot catalog (SPEC §12.12–12.13).

use std::sync::OnceLock;

use pawprint_sprite::{Bundle, Library, Mascot};

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

/// The mascots a badge can show, with their alias table.
#[derive(Debug, Clone)]
pub struct Catalog {
    mascots: Vec<Mascot>,
    aliases: AliasTable,
}

impl Catalog {
    pub fn new(mascots: Vec<Mascot>) -> Catalog {
        let aliases = AliasTable::new(
            mascots
                .iter()
                .map(|m| (m.id.as_str(), m.aliases.as_slice())),
        );
        Catalog { mascots, aliases }
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

    pub fn aliases(&self) -> &AliasTable {
        &self.aliases
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
    }

    #[test]
    fn user_text_mascots_can_use_the_embedded_library() {
        let src = r##"
id = "my-cat"
name = "My Cat"
category = "cats-dogs"
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
        let c = pawprint_sprite::compile(
            src,
            Some("my-cat"),
            library(),
            pawprint_sprite::Origin::User,
            &|n| Err(n.to_string()),
        );
        assert!(c.report.errors.is_empty(), "{:?}", c.report.errors);
        let m = c.mascot.unwrap();
        assert_eq!(m.base.as_deref(), Some("cat-sit"));
        assert!(m.colors.contains(&pawprint_sprite::Rgb(0xc0, 0x80, 0x40)));
    }

    #[test]
    fn bundle_has_font_and_crate() {
        let b = bundle();
        assert_eq!(b.font.glyphs.len(), 95);
        assert_eq!(b.crate_art.front.height, 12);
        assert_eq!(b.minimum, 100);
    }
}
