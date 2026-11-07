//! `pawdev`: PAWPRINT's developer tool. Never shipped. Every `make` target
//! that needs real logic runs through here, so recipes stay single commands
//! that behave the same under `sh` and `cmd.exe` (SPEC §14.1).

mod art;
mod golden;
mod power;
mod sheet;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: pawdev <command>

commands:
  help <Makefile>          list make targets and their ## descriptions
  check-assets [--package] validate assets/; --package also enforces the catalog gates
  golden                   regenerate tests/golden/*.png
  palette                  write assets/palette.gpl from assets/palette.toml
  contact-sheet [--animated] docs/gallery.png (and docs/gallery.gif)
  building-blocks          docs/building-blocks.png: accents, props, bases x templates
  art-export <id>          out/art/<id>/: a PNG per layer and variant, plus frames.png
  art-import <id> <png>    print a PNG layer as a [layers] table in <id>'s glyphs
  power-check [seconds]    measure the release badge against SPEC §7.2 (Linux, Xvfb)
  todo <target> <milestone> report a make target that is not implemented yet
  clean-dist               remove dist/";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["help", makefile] => help(Path::new(makefile)),
        ["check-assets"] => check_assets(false),
        ["check-assets", "--package"] => check_assets(true),
        ["golden"] => golden::write_all(&root()),
        ["palette"] => art::write_palette_gpl(&root()),
        ["contact-sheet"] => sheet::contact_sheet(&root(), false),
        ["contact-sheet", "--animated"] => sheet::contact_sheet(&root(), true),
        ["building-blocks"] => sheet::building_blocks(&root()),
        ["art-export", id] => art::export(&root(), id),
        ["art-import", id, png] => art::import(&root(), id, Path::new(png)),
        ["art-import", _] => Err("usage: make art-import MASCOT=<id> PNG=<file>".into()),
        ["power-check"] => power::power_check(&root(), None),
        ["power-check", seconds] => power::power_check(&root(), Some(seconds)),
        ["todo", target, milestone] => Err(format!(
            "make {target}: not implemented yet ({milestone}); see ROADMAP.md"
        )),
        ["clean-dist"] => clean_dist(),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("tools/pawdev is two levels deep")
        .to_path_buf()
}

/// Print `target  description` for every `target: ... ## description` line.
fn help(makefile: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(makefile)
        .map_err(|e| format!("cannot read {}: {e}", makefile.display()))?;
    let targets = parse_help(&text);
    let width = targets.iter().map(|(t, _)| t.len()).max().unwrap_or(0);
    println!("PAWPRINT make targets:\n");
    for (target, desc) in targets {
        println!("  make {target:<width$}  {desc}");
    }
    Ok(())
}

fn parse_help(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter_map(|line| {
            let (rule, desc) = line.split_once("## ")?;
            let (target, _) = rule.split_once(':')?;
            let ok = !target.is_empty()
                && target
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            ok.then(|| (target, desc.trim()))
        })
        .collect()
}

fn check_assets(package: bool) -> Result<(), String> {
    let built = pawprint_sprite::build_bundle(&root().join("assets"));
    for w in &built.report.warnings {
        println!("warning: {w}");
    }
    for e in &built.report.errors {
        println!("error: {e}");
    }
    let Some(bundle) = built.bundle else {
        return Err(format!(
            "check-assets: {} error(s)",
            built.report.errors.len()
        ));
    };
    println!(
        "assets OK: {} mascot(s), {} accents, font and crate valid",
        bundle.mascots.len(),
        bundle.accents.len()
    );
    let gates = pawprint_sprite::check_gates(&bundle);
    if gates.is_empty() {
        println!("catalog gates: pass");
        return Ok(());
    }
    if package {
        for g in &gates {
            println!("gate: {g}");
        }
        return Err(format!(
            "catalog gates: {} failure(s); `make package` needs a complete catalog (SPEC §12.11 rule 9)",
            gates.len()
        ));
    }
    println!(
        "catalog gates: {} not met yet (only `make package` enforces them)",
        gates.len()
    );
    Ok(())
}

fn clean_dist() -> Result<(), String> {
    let dist = root().join("dist");
    match std::fs::remove_dir_all(&dist) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", dist.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_parses_described_targets_only() {
        let text = "CARGO ?= cargo\nhelp: ## List targets\n\t@$(PAWDEV) help\nci: lint test ## lint + test\n.PHONY: help\nnodesc:\n\techo\n# comment ## not a target\n";
        assert_eq!(
            parse_help(text),
            vec![("help", "List targets"), ("ci", "lint + test")]
        );
    }

    #[test]
    fn every_make_target_has_a_description() {
        let text = std::fs::read_to_string(root().join("Makefile")).unwrap();
        let described: Vec<&str> = parse_help(&text).into_iter().map(|(t, _)| t).collect();
        // SPEC §14.2: every public target exists.
        for t in [
            "help",
            "build",
            "run",
            "preview",
            "test",
            "check-assets",
            "lint",
            "fmt",
            "golden",
            "contact-sheet",
            "art-export",
            "art-import",
            "building-blocks",
            "palette",
            "ci",
            "release",
            "package",
            "publish",
            "install",
            "uninstall",
            "doctor",
            "clean",
        ] {
            assert!(
                described.contains(&t),
                "make {t} is missing or has no ## description"
            );
        }
        let readme = std::fs::read_to_string(root().join("README.md")).unwrap();
        for t in &described {
            assert!(
                readme.contains(&format!("make {t}")),
                "README's Development section doesn't mention `make {t}`"
            );
        }
    }
}
