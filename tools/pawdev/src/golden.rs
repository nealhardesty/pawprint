//! Golden badge renders and the catalog stability fixture (SPEC §16.1).
//! `make golden` rewrites the renders and `make golden-hosts` the fixture.
//! Render tests compare decoded pixels, so a PNG encoder change can't cause
//! a false failure but any rendering change does.

use std::path::Path;

use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::config::{Config, Layer};
use pawprint_core::identity;
use pawprint_core::layout::{Anchor, LabelSize};
use pawprint_core::render::{BadgeSpec, render_badge};
use pawprint_core::sprite::{Rgb, RgbaImage};
use pawprint_core::state::State;

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

/// The catalog stability fixture (SPEC §16.1): synthetic hostnames and the
/// mascot each gets from the bundled catalog on a fresh install.
const HOSTS: &str = "tests/golden/hostnames.tsv";

const HOSTS_HEADER: &str = "\
# Catalog stability golden test (SPEC §16.1): the mascot each hostname gets
# from the bundled catalog on a fresh install (no config, no state), and the
# rule that chose it. A change here changes real machines' mascots, so
# regenerate only on purpose, with `make golden-hosts`, and review the diff.
# To add a hostname, append it on its own line and regenerate.
";

/// One fixture line for `host`.
fn host_line(host: &str) -> String {
    let r = identity::resolve(
        host,
        &Config::default(),
        &State::default(),
        &Layer::default(),
        Catalog::bundled(),
        &bundle().accents,
        false,
    );
    format!("{host:<24}{:<24}{}", r.mascot, r.rule.number())
}

/// The fixture's hostnames: the first word of each non-comment line.
fn fixture_hosts(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .collect()
}

fn render_hosts(text: &str) -> String {
    let lines: Vec<String> = fixture_hosts(text).into_iter().map(host_line).collect();
    format!("{HOSTS_HEADER}{}\n", lines.join("\n"))
}

/// `make golden-hosts`: recompute every hostname's mascot in the fixture.
pub fn write_hosts(root: &Path) -> Result<(), String> {
    let path = root.join(HOSTS);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&path, render_hosts(&text)).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "wrote {} ({} hostnames)",
        path.display(),
        fixture_hosts(&text).len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_keep_their_mascots() {
        let path = crate::root().join(HOSTS);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(fixture_hosts(&text).len(), 200);
        let want: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        let fresh = render_hosts(&text);
        let got: Vec<&str> = fresh.lines().filter(|l| !l.starts_with('#')).collect();
        let changed: Vec<String> = want
            .iter()
            .zip(&got)
            .filter(|(w, g)| w != g)
            .map(|(w, g)| format!("  was {w}\n  now {g}"))
            .collect();
        assert!(
            changed.is_empty(),
            "the catalog moves {} of {} hostnames to a new mascot:\n{}\n\
             If that is intended, run `make golden-hosts` and commit the fixture.",
            changed.len(),
            want.len(),
            changed.join("\n")
        );
    }

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
