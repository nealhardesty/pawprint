//! Compile and validate every bundled asset with `pawprint-sprite`, then
//! embed the result (SPEC §12.13). Any error or warning fails the build.
//! Catalog gates (≥100 mascots, required ids) are NOT checked here; only
//! `make package` enforces them (SPEC §12.11 rule 9).

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let assets = manifest.join("../../assets");
    println!("cargo:rerun-if-changed={}", assets.display());

    let built = pawprint_sprite::build_bundle(&assets);
    if !built.report.errors.is_empty() {
        for e in &built.report.errors {
            eprintln!("asset error: {e}");
        }
        eprintln!(
            "{} asset error(s); run `make check-assets` for the full report",
            built.report.errors.len()
        );
        std::process::exit(1);
    }
    let bundle = built.bundle.expect("no errors means a bundle");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo")).join("bundle.bin");
    std::fs::write(&out, bundle.to_bytes()).expect("write bundle.bin");
}
