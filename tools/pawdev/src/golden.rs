//! Golden badge renders (SPEC §16.1). `make golden` rewrites them; the test
//! compares decoded pixels, so a PNG encoder change can't cause a false
//! failure but any rendering change does.

use std::path::Path;

use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::layout::{Anchor, LabelSize};
use pawprint_core::render::{BadgeSpec, render_badge};
use pawprint_core::sprite::{Rgb, RgbaImage};

/// (file stem, mascot, frame, size, label)
pub const CASES: &[(&str, &str, usize, u32, Option<&str>)] = &[
    ("owl-32", "owl", 0, 32, Some("OWL")),
    ("owl-64", "owl", 0, 64, Some("OWL")),
    ("owl-128", "owl", 0, 128, Some("OWL")),
    ("owl-64-blink", "owl", 1, 64, None),
    ("owl-64-nameplate", "owl", 0, 64, Some("PROD-OWL-02")),
    ("owl-32-nameplate-beside", "owl", 0, 32, Some("PROD-OWL-02")),
];

const VIOLET: Rgb = Rgb(0x7b, 0x5e, 0xa7);

pub fn render(mascot: &str, frame: usize, size: u32, label: Option<&str>) -> RgbaImage {
    let spec = BadgeSpec {
        mascot: Catalog::bundled()
            .get(mascot)
            .expect("golden mascots are bundled"),
        frame,
        label,
        accent: VIOLET,
        size,
        label_size: LabelSize::Auto,
        anchor: Anchor::TopRight,
        display_scale: 1.0,
    };
    render_badge(&spec, &bundle().crate_art, &bundle().font)
}

pub fn write_all(root: &Path) -> Result<(), String> {
    let dir = root.join("tests/golden");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (stem, mascot, frame, size, label) in CASES {
        let path = dir.join(format!("{stem}.png"));
        std::fs::write(&path, render(mascot, *frame, *size, *label).encode_png())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_match_golden_images() {
        let dir = crate::root().join("tests/golden");
        for (stem, mascot, frame, size, label) in CASES {
            let path = dir.join(format!("{stem}.png"));
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{}: {e}; run `make golden`", path.display()));
            let want = RgbaImage::decode_png(&bytes).unwrap();
            let got = render(mascot, *frame, *size, *label);
            assert!(
                got == want,
                "{stem} differs from {}; if the change is intended, run `make golden`",
                path.display()
            );
        }
    }
}
