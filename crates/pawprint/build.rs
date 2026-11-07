//! Bake the git commit and target triple into `pawprint --version`.

fn main() {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=PAWPRINT_GIT_COMMIT={commit}");
    println!(
        "cargo:rustc-env=PAWPRINT_TARGET={}",
        std::env::var("TARGET").expect("set by cargo")
    );
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}
