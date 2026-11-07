//! `make release`, `make package`, and `make publish` (SPEC §14.5, §15).
//!
//! - release: optimized binaries for this OS. Linux builds static musl for
//!   x86_64 and aarch64 (rust-lld, F22); macOS builds both Darwin targets,
//!   joins them with `lipo`, and re-signs ad hoc (F15); Windows builds
//!   `pawprint.exe` and `pawprintw.exe`. Each binary is checked (size budget,
//!   static linking, signature) and run where this machine can run it.
//! - package: release, then one archive per build plus a `sha256sum`-format
//!   sidecar into `dist/`, named exactly as SPEC §15.2 says.
//!   `dist/COMMIT` records the commit they were built from.
//! - publish: upload `dist/*` and the installers to the GitHub release
//!   `v<version>` with `gh`, creating it as a draft on first use.
//! - finalize: once every OS has uploaded, check that every asset is there
//!   and publish the draft.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::root;

/// SPEC §7.2: every mascot embedded, under 15 MB.
const BINARY_BUDGET: u64 = 15_000_000;

/// One release archive: the target in its asset name and the binaries in it.
pub struct Build {
    /// `x86_64-unknown-linux-musl`, `universal-apple-darwin`, ...
    pub target: &'static str,
    pub bins: Vec<PathBuf>,
}

impl Build {
    fn new(root: &Path, target: &'static str, names: &[&str]) -> Build {
        let dir = root.join("target").join(target).join("release");
        Build {
            target,
            bins: names.iter().map(|n| dir.join(n)).collect(),
        }
    }

    pub fn is_zip(&self) -> bool {
        self.target.contains("windows")
    }

    /// The asset name (SPEC §15.2).
    pub fn archive_name(&self, version: &str) -> String {
        archive_name(self.target, version)
    }
}

pub fn archive_name(target: &str, version: &str) -> String {
    let ext = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("pawprint-{version}-{target}.{ext}")
}

/// The cargo targets this OS builds, and the archives they become.
fn plan(root: &Path) -> (Vec<&'static str>, Vec<Build>) {
    if cfg!(target_os = "macos") {
        let darwin = vec!["aarch64-apple-darwin", "x86_64-apple-darwin"];
        (
            darwin,
            vec![Build::new(root, "universal-apple-darwin", &["pawprint"])],
        )
    } else if cfg!(windows) {
        let t = "x86_64-pc-windows-msvc";
        (
            vec![t],
            vec![Build::new(root, t, &["pawprint.exe", "pawprintw.exe"])],
        )
    } else {
        let musl = vec!["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];
        let builds = musl
            .iter()
            .map(|t| Build::new(root, t, &["pawprint"]))
            .collect();
        (musl, builds)
    }
}

/// The archive `make install` uses on this machine.
pub fn native_build(root: &Path) -> Build {
    let (_, builds) = plan(root);
    let single = builds.len() == 1;
    builds
        .into_iter()
        .find(|b| single || b.target.starts_with(std::env::consts::ARCH))
        .expect("one build per OS, or one per Linux architecture")
}

/// `[workspace.package]` in the root `Cargo.toml`: (version, "owner/repo").
pub fn workspace_package(root: &Path) -> Result<(String, String), String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let doc: toml::Table = text.parse().map_err(|e| format!("Cargo.toml: {e}"))?;
    let pkg = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .ok_or("Cargo.toml has no [workspace.package]")?;
    let get = |key: &str| {
        pkg.get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or(format!("[workspace.package] has no {key}"))
    };
    let repo = get("repository")?;
    let slug = repo
        .strip_prefix("https://github.com/")
        .ok_or(format!("repository {repo} is not on github.com"))?
        .trim_end_matches('/')
        .to_string();
    Ok((get("version")?, slug))
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

/// Run a command; fail with its name if it can't start or exits non-zero.
pub fn run(cmd: &mut Command) -> Result<(), String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run {name}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} failed ({status})"))
    }
}

/// Run a command and return its stdout, or why it failed.
pub fn output(cmd: &mut Command) -> Result<String, String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {name}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "{name} failed ({}): {}{}",
            out.status,
            text,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The first `name` on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&exe))
        .find(|p| p.is_file())
}

/// `make release`.
pub fn release() -> Result<Vec<Build>, String> {
    let root = root();
    let (version, _) = workspace_package(&root)?;
    let (targets, builds) = plan(&root);
    println!("building pawprint {version} for {}...", targets.join(", "));
    let mut cmd = cargo();
    cmd.args(["build", "--release", "--package", "pawprint"])
        .current_dir(&root);
    for t in &targets {
        cmd.args(["--target", t]);
    }
    run(&mut cmd)?;
    if cfg!(target_os = "macos") {
        universal(&root, &targets, &builds[0].bins[0])?;
    }
    println!();
    for build in &builds {
        for bin in &build.bins {
            verify(build.target, bin, &version)?;
        }
    }
    Ok(builds)
}

/// macOS: one universal binary from both slices, re-signed ad hoc, because
/// `lipo` invalidates the linker's signature and Apple Silicon runs only
/// signed arm64 code (SPEC §11.2, F15).
fn universal(root: &Path, targets: &[&str], out: &Path) -> Result<(), String> {
    fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut lipo = Command::new("lipo");
    lipo.arg("-create").arg("-output").arg(out);
    for t in targets {
        lipo.arg(root.join("target").join(t).join("release/pawprint"));
    }
    run(&mut lipo)?;
    run(Command::new("codesign")
        .args(["--force", "-s", "-"])
        .arg(out))
}

/// Check one release binary and say what was checked.
fn verify(target: &str, bin: &Path, version: &str) -> Result<(), String> {
    let size = fs::metadata(bin)
        .map_err(|e| format!("{}: {e}", bin.display()))?
        .len();
    let name = bin.file_name().unwrap().to_string_lossy();
    let mut notes = vec![format!("{:.2} MB", size as f64 / 1e6)];
    if size >= BINARY_BUDGET {
        return Err(format!(
            "{name} for {target} is {size} bytes, over the 15 MB budget (SPEC §7.2)"
        ));
    }
    if target.contains("linux") {
        if has_interpreter(bin)? {
            return Err(format!(
                "{} is dynamically linked (it has PT_INTERP); the Linux release must be static (SPEC §14.4)",
                bin.display()
            ));
        }
        notes.push("static (no PT_INTERP)".into());
    }
    if target.contains("apple") {
        run(Command::new("codesign").arg("-v").arg(bin))
            .map_err(|e| format!("{}: signature check failed: {e}", bin.display()))?;
        let archs = output(Command::new("lipo").arg("-archs").arg(bin))?;
        if !(archs.contains("x86_64") && archs.contains("arm64")) {
            return Err(format!("{} is not universal: {archs}", bin.display()));
        }
        notes.push(format!("codesign -v ok, lipo -archs: {archs}"));
    }
    match runner(target) {
        Some(emulator) => {
            let mut cmd = match &emulator {
                Some(qemu) => {
                    let mut c = Command::new(qemu);
                    c.arg(bin);
                    c
                }
                None => Command::new(bin),
            };
            let out = output(cmd.arg("--version"))?;
            if !out.starts_with(&format!("pawprint {version} (")) {
                return Err(format!("{} --version said {out:?}", bin.display()));
            }
            let how = emulator.map_or(String::new(), |q| {
                format!(" under {}", q.file_name().unwrap().to_string_lossy())
            });
            notes.push(format!("ran{how}: {out}"));
        }
        None => notes.push("not run here (install qemu-user-static to run it)".into()),
    }
    if target.contains("apple") && cfg!(target_arch = "aarch64") {
        // Informational: the x86_64 slice runs only where Rosetta 2 is installed.
        let x86 = output(
            Command::new("arch")
                .arg("-x86_64")
                .arg(bin)
                .arg("--version"),
        );
        notes.push(match x86 {
            Ok(_) => "x86_64 slice ran under Rosetta".into(),
            Err(_) => "x86_64 slice not run (no Rosetta)".into(),
        });
    }
    println!("  {target:<28} {name:<14} {}", notes.join(", "));
    Ok(())
}

/// How this machine can run a binary for `target`: `Some(None)` natively,
/// `Some(Some(qemu))` under user-mode emulation, `None` not at all.
fn runner(target: &str) -> Option<Option<PathBuf>> {
    if target.starts_with("universal") || target.starts_with(std::env::consts::ARCH) {
        return Some(None);
    }
    if target.starts_with("aarch64") && cfg!(target_os = "linux") {
        return ["qemu-aarch64-static", "qemu-aarch64"]
            .iter()
            .find_map(|q| which(q))
            .map(Some);
    }
    None
}

/// Whether an ELF64 executable names a dynamic loader (`PT_INTERP`).
/// Static and static-pie executables don't.
fn has_interpreter(bin: &Path) -> Result<bool, String> {
    let data = fs::read(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    elf_has_interpreter(&data).ok_or(format!("{} is not an ELF64 file", bin.display()))
}

fn elf_has_interpreter(data: &[u8]) -> Option<bool> {
    if data.get(..5)? != b"\x7fELF\x02" || data.get(5)? != &1 {
        return None; // not 64-bit little-endian ELF
    }
    let u16_at = |o: usize| Some(u16::from_le_bytes(data.get(o..o + 2)?.try_into().ok()?));
    let u64_at = |o: usize| Some(u64::from_le_bytes(data.get(o..o + 8)?.try_into().ok()?));
    let phoff = usize::try_from(u64_at(0x20)?).ok()?;
    let phentsize = usize::from(u16_at(0x36)?);
    let phnum = usize::from(u16_at(0x38)?);
    const PT_INTERP: u32 = 3;
    for i in 0..phnum {
        let o = phoff + i * phentsize;
        let p_type = u32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?);
        if p_type == PT_INTERP {
            return Some(true);
        }
    }
    Some(false)
}

/// `make package`.
pub fn package() -> Result<(), String> {
    let builds = release()?;
    let root = root();
    let (version, _) = workspace_package(&root)?;
    let dist = root.join("dist");
    fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let commit = built_from(&root)?;
    fs::write(dist.join(COMMIT_FILE), format!("{commit}\n")).map_err(|e| e.to_string())?;
    let mtime = source_date_epoch(&root);
    println!();
    for build in &builds {
        let name = build.archive_name(&version);
        let path = dist.join(&name);
        let mut files: Vec<(String, PathBuf, u32)> = build
            .bins
            .iter()
            .map(|b| {
                (
                    b.file_name().unwrap().to_string_lossy().into(),
                    b.clone(),
                    0o755,
                )
            })
            .collect();
        for doc in ["README.md", "LICENSE"] {
            files.push((doc.into(), root.join(doc), 0o644));
        }
        if build.is_zip() {
            write_zip(&path, &files, mtime)?;
        } else {
            write_tar_gz(&path, &files, mtime)?;
        }
        let hash = write_sidecar(&path)?;
        let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        println!("  dist/{name}  {:.2} MB  sha256 {hash}", size as f64 / 1e6);
    }
    println!("  dist/{COMMIT_FILE}  {commit}");
    Ok(())
}

/// `dist/COMMIT`: the commit `make package` built from, so `make publish`
/// can refuse archives built from anything but the tagged commit.
const COMMIT_FILE: &str = "COMMIT";

/// `HEAD`, with `-dirty` appended when the tree has uncommitted changes.
fn built_from(root: &Path) -> Result<String, String> {
    let git = |args: &[&str]| output(Command::new("git").args(args).current_dir(root));
    let head = git(&["rev-parse", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain"])?.is_empty();
    Ok(if dirty { format!("{head}-dirty") } else { head })
}

/// `dist/` must have been packaged from `head` (the tagged commit).
fn check_dist_commit(root: &Path, head: &str) -> Result<(), String> {
    let commit = fs::read_to_string(root.join("dist").join(COMMIT_FILE)).map_err(|_| {
        format!(
            "dist/{COMMIT_FILE} is missing, so dist/ can't be matched to a commit; run `make clean` and `make package`"
        )
    })?;
    let commit = commit.trim();
    if commit != head {
        return Err(format!(
            "dist/ was packaged from {commit}, but the tag is on {head}; run `make clean` and `make package` from the tag"
        ));
    }
    Ok(())
}

/// The archives' file times: `SOURCE_DATE_EPOCH` if set, else the commit
/// time of `HEAD`, so packaging a commit twice gives the same archive bytes
/// (from the same binaries).
fn source_date_epoch(root: &Path) -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .or_else(|| {
            output(
                Command::new("git")
                    .args(["log", "-1", "--format=%ct", "HEAD"])
                    .current_dir(root),
            )
            .ok()
        })
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(now)
}

type Entry = (String, PathBuf, u32);

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A flat `.tar.gz`: the files at the top level, owned by root, with the
/// given modes and modification time (seconds since 1970).
pub fn write_tar_gz(path: &Path, files: &[Entry], mtime: u64) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    let gz = flate2::write::GzEncoder::new(
        File::create(path).map_err(fail)?,
        flate2::Compression::best(),
    );
    let mut tar = tar::Builder::new(gz);
    for (name, src, mode) in files {
        let data = read(src)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(*mode);
        header.set_mtime(mtime);
        header.set_cksum();
        tar.append_data(&mut header, name, data.as_slice())
            .map_err(fail)?;
    }
    tar.into_inner().map_err(fail)?.finish().map_err(fail)?;
    Ok(())
}

/// A flat `.zip` (deflated), every file stamped with `mtime` (as UTC).
pub fn write_zip(path: &Path, files: &[Entry], mtime: u64) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("cannot write {}: {e}", path.display());
    let time = zip_time(mtime).map_err(|e| fail(&e))?;
    let mut zip = zip::ZipWriter::new(File::create(path).map_err(|e| fail(&e))?);
    for (name, src, mode) in files {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(time)
            .unix_permissions(*mode);
        zip.start_file(name.as_str(), options)
            .map_err(|e| fail(&e))?;
        zip.write_all(&read(src)?).map_err(|e| fail(&e))?;
    }
    zip.finish().map_err(|e| fail(&e))?;
    Ok(())
}

/// Seconds since 1970 as a zip (DOS) time, which can't go before 1980.
fn zip_time(secs: u64) -> Result<zip::DateTime, String> {
    // Days to a civil date: Howard Hinnant's `civil_from_days`.
    let days = (secs / 86400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    if year < 1980 {
        return Ok(zip::DateTime::default());
    }
    let t = secs % 86400;
    let (h, m, s) = ((t / 3600) as u8, (t / 60 % 60) as u8, (t % 60) as u8);
    zip::DateTime::from_date_and_time(year, month, day, h, m, s)
        .map_err(|e| format!("{secs} is not a zip time: {e}"))
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Write `<archive>.sha256` in `sha256sum` format and return the hash.
fn write_sidecar(archive: &Path) -> Result<String, String> {
    let hash = sha256_hex(&read(archive)?);
    let name = archive.file_name().unwrap().to_string_lossy();
    let sidecar = archive.with_file_name(format!("{name}.sha256"));
    fs::write(&sidecar, format!("{hash}  {name}\n"))
        .map_err(|e| format!("cannot write {}: {e}", sidecar.display()))?;
    Ok(hash)
}

/// Every archive a release carries, one per OS build (SPEC §15.2).
const RELEASE_TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "universal-apple-darwin",
    "x86_64-pc-windows-msvc",
];

const INSTALLERS: [&str; 2] = ["installers/install.sh", "installers/install.ps1"];

fn gh_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn gh() -> Result<PathBuf, String> {
    which("gh").ok_or_else(|| {
        "this needs the GitHub CLI: https://cli.github.com, then `gh auth login`".to_string()
    })
}

/// `make publish` (`DRY_RUN=1` prints the commands and runs nothing).
pub fn publish(dry_run: bool) -> Result<(), String> {
    let root = root();
    let (version, repo) = workspace_package(&root)?;
    let tag = format!("v{version}");
    let assets = publish_assets(&root, &version)?;
    let ready = check_tree(&root, &tag).and_then(|head| check_dist_commit(&root, &head));

    let title = format!("PAWPRINT {version}");
    let view = gh_args(&["release", "view", &tag, "--repo", &repo]);
    let create = gh_args(&[
        "release",
        "create",
        &tag,
        "--repo",
        &repo,
        "--draft",
        "--title",
        &title,
        "--generate-notes",
        "--verify-tag",
    ]);
    let mut upload = gh_args(&["release", "upload", &tag, "--repo", &repo, "--clobber"]);
    upload.extend(assets);

    if dry_run {
        if let Err(e) = &ready {
            println!("note: a real publish would refuse right now:\n  {e}\n");
        }
        println!("DRY_RUN: would run, from {}:", root.display());
        println!("  {}    # does the release exist?", shell_line(&view));
        println!("  {}    # only if it doesn't", shell_line(&create));
        println!("  {}", shell_line(&upload));
        return Ok(());
    }
    ready?;
    let gh = gh()?;
    let exists = Command::new(&gh)
        .args(&view)
        .current_dir(&root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if exists {
        println!("release {tag} exists; uploading (same-named assets are replaced)");
    } else {
        println!("creating draft release {tag}");
        run(Command::new(&gh).args(&create).current_dir(&root))?;
    }
    run(Command::new(&gh).args(&upload).current_dir(&root))?;
    println!(
        "uploaded to https://github.com/{repo}/releases/tag/{tag}\n\
         once Linux, macOS, and Windows have all published, run `make release-finalize`"
    );
    Ok(())
}

/// `make release-finalize`: check that the draft release has every asset of
/// every OS, then publish it (`DRY_RUN=1` prints the commands only).
pub fn finalize(dry_run: bool) -> Result<(), String> {
    let root = root();
    let (version, repo) = workspace_package(&root)?;
    let tag = format!("v{version}");
    let expected = release_assets(&version);
    let list = gh_args(&[
        "release",
        "view",
        &tag,
        "--repo",
        &repo,
        "--json",
        "assets",
        "--jq",
        ".assets[].name",
    ]);
    let edit = gh_args(&["release", "edit", &tag, "--repo", &repo, "--draft=false"]);

    if dry_run {
        println!("DRY_RUN: would run, from {}:", root.display());
        println!("  {}    # the uploaded assets", shell_line(&list));
        println!(
            "  {}    # only if every asset below is there",
            shell_line(&edit)
        );
        println!("expected assets:");
        for a in &expected {
            println!("  {a}");
        }
        return Ok(());
    }
    let gh = gh()?;
    let present = output(Command::new(&gh).args(&list).current_dir(&root))?;
    let missing = missing_assets(&expected, &present);
    if !missing.is_empty() {
        return Err(format!(
            "release {tag} is still missing:\n  {}\nrun `make package && make publish` on the OS that builds them, then try again",
            missing.join("\n  ")
        ));
    }
    run(Command::new(&gh).args(&edit).current_dir(&root))?;
    println!(
        "published: https://github.com/{repo}/releases/tag/{tag} (all {} assets)",
        expected.len()
    );
    Ok(())
}

/// The asset names a complete release has: each archive, its sidecar, and
/// the installers.
fn release_assets(version: &str) -> Vec<String> {
    let mut names = Vec::new();
    for target in RELEASE_TARGETS {
        let archive = archive_name(target, version);
        names.push(format!("{archive}.sha256"));
        names.push(archive);
    }
    names.extend(INSTALLERS.map(|p| p.rsplit('/').next().unwrap().to_string()));
    names
}

/// The expected names not among `present` (one name per line).
fn missing_assets(expected: &[String], present: &str) -> Vec<String> {
    let present: Vec<&str> = present.lines().map(str::trim).collect();
    expected
        .iter()
        .filter(|name| !present.contains(&name.as_str()))
        .cloned()
        .collect()
}

/// `dist/*` (all of this version) and the two installers, relative to the root.
fn publish_assets(root: &Path, version: &str) -> Result<Vec<String>, String> {
    let dist = root.join("dist");
    let mut files: Vec<String> = fs::read_dir(&dist)
        .map_err(|_| "dist/ is empty: run `make package` first".to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f != COMMIT_FILE)
        .collect();
    files.sort();
    if files.is_empty() {
        return Err("dist/ is empty: run `make package` first".into());
    }
    let prefix = format!("pawprint-{version}-");
    if let Some(stale) = files.iter().find(|f| !f.starts_with(&prefix)) {
        return Err(format!(
            "dist/{stale} is not from version {version}; run `make clean` and `make package`"
        ));
    }
    let mut assets: Vec<String> = files.iter().map(|f| format!("dist/{f}")).collect();
    assets.extend(INSTALLERS.map(String::from));
    Ok(assets)
}

/// A release must be built from exactly the tagged commit: a clean tree, and
/// the tag on HEAD. Returns HEAD.
pub fn check_tree(root: &Path, tag: &str) -> Result<String, String> {
    let git = |args: &[&str]| output(Command::new("git").args(args).current_dir(root));
    let dirty = git(&["status", "--porcelain"])?;
    if !dirty.is_empty() {
        return Err(format!(
            "the working tree has uncommitted changes, so the release wouldn't match a commit; commit or remove them first:\n{dirty}"
        ));
    }
    let head = git(&["rev-parse", "HEAD"])?;
    let tagged = git(&["rev-parse", "--verify", "--quiet", &format!("refs/tags/{tag}^{{commit}}")])
        .map_err(|_| {
            format!(
                "tag {tag} doesn't exist; tag this commit and push it: git tag {tag} && git push origin {tag}"
            )
        })?;
    if tagged != head {
        return Err(format!(
            "tag {tag} points at {}, but HEAD is {}; check out the tag (git checkout {tag}) or bump the version",
            &tagged[..12.min(tagged.len())],
            &head[..12.min(head.len())]
        ));
    }
    Ok(head)
}

/// Quote arguments for display, so a printed command can be pasted.
fn shell_line(args: &[String]) -> String {
    std::iter::once("gh".to_string())
        .chain(args.iter().map(|a| {
            if a.contains(|c: char| c.is_whitespace() || "\"'$[]*?".contains(c)) {
                format!("'{a}'")
            } else {
                a.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_match_spec() {
        assert_eq!(
            archive_name("x86_64-pc-windows-msvc", "1.2.0"),
            "pawprint-1.2.0-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            archive_name("universal-apple-darwin", "1.2.0"),
            "pawprint-1.2.0-universal-apple-darwin.tar.gz"
        );
        assert_eq!(
            archive_name("aarch64-unknown-linux-musl", "1.2.0"),
            "pawprint-1.2.0-aarch64-unknown-linux-musl.tar.gz"
        );
    }

    #[test]
    fn version_and_repo_come_from_the_workspace() {
        let (version, repo) = workspace_package(&root()).unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert_eq!(repo, "nealhardesty/pawprint");
    }

    #[test]
    fn native_build_is_one_of_this_os() {
        let b = native_build(&root());
        assert!(b.target.starts_with(std::env::consts::ARCH) || b.target.starts_with("universal"));
    }

    #[test]
    fn elf_interpreter() {
        assert_eq!(elf_has_interpreter(b"MZ"), None);
        // A minimal ELF64 header with one PT_INTERP program header.
        let mut elf = vec![0u8; 0x40 + 56];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
        elf[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        elf[0x38..0x3a].copy_from_slice(&1u16.to_le_bytes());
        elf[0x40..0x44].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(elf_has_interpreter(&elf), Some(true));
        elf[0x40..0x44].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        assert_eq!(elf_has_interpreter(&elf), Some(false));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pawdev-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn archives_and_sidecar() {
        let dir = temp("archives");
        let bin = dir.join("pawprint");
        fs::write(&bin, b"binary").unwrap();
        let files = vec![
            ("pawprint".to_string(), bin.clone(), 0o755),
            ("LICENSE".to_string(), root().join("LICENSE"), 0o644),
        ];

        let tgz = dir.join("a.tar.gz");
        write_tar_gz(&tgz, &files, 1_700_000_000).unwrap();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(&tgz).unwrap()));
        let entries: Vec<(String, u32)> = tar
            .entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().unwrap().to_string_lossy().into(),
                    e.header().mode().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            entries,
            vec![("pawprint".into(), 0o755), ("LICENSE".into(), 0o644)]
        );

        let zip_path = dir.join("a.zip");
        write_zip(&zip_path, &files, 1_700_000_000).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        // 1700000000 is 2023-11-14 22:13:20 UTC.
        let time = zip.by_index(0).unwrap().last_modified().unwrap();
        assert_eq!((time.year(), time.month(), time.day()), (2023, 11, 14));
        assert_eq!((time.hour(), time.minute(), time.second()), (22, 13, 20));
        let mut data = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("pawprint").unwrap(), &mut data).unwrap();
        assert_eq!(data, "binary");

        let hash = write_sidecar(&tgz).unwrap();
        let line = fs::read_to_string(dir.join("a.tar.gz.sha256")).unwrap();
        assert_eq!(line, format!("{hash}  a.tar.gz\n"));
        assert_eq!(hash, sha256_hex(&fs::read(&tgz).unwrap()));
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repackaging_gives_the_same_bytes() {
        let dir = temp("reproducible");
        let files = vec![("LICENSE".to_string(), root().join("LICENSE"), 0o644)];
        for ext in ["tar.gz", "zip"] {
            let (a, b) = (dir.join(format!("a.{ext}")), dir.join(format!("b.{ext}")));
            for path in [&a, &b] {
                if ext == "zip" {
                    write_zip(path, &files, 1_700_000_000).unwrap();
                } else {
                    write_tar_gz(path, &files, 1_700_000_000).unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(1100));
            }
            assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap(), "{ext}");
        }
        assert_eq!(zip_time(0).unwrap(), zip::DateTime::default());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_complete_release_has_every_archive_sidecar_and_installer() {
        let expected = release_assets("1.2.0");
        assert_eq!(expected.len(), 10);
        for target in RELEASE_TARGETS {
            let archive = archive_name(target, "1.2.0");
            assert!(expected.contains(&format!("{archive}.sha256")));
            assert!(expected.contains(&archive));
        }
        // This OS's builds are among them.
        for build in plan(&root()).1 {
            assert!(RELEASE_TARGETS.contains(&build.target), "{}", build.target);
        }
        let all = expected.join("\n");
        assert!(missing_assets(&expected, &all).is_empty());
        let partial = format!(
            "install.sh\n{}\n",
            archive_name(RELEASE_TARGETS[0], "1.2.0")
        );
        let missing = missing_assets(&expected, &partial);
        assert_eq!(missing.len(), 8);
        assert!(missing.contains(&"install.ps1".to_string()));
        assert!(!missing.contains(&"install.sh".to_string()));
    }

    #[test]
    fn publish_refuses_a_dirty_tree_or_a_tag_off_head() {
        let dir = temp("publish");
        let git = |args: &[&str]| {
            output(
                Command::new("git")
                    .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                    .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
                    .args(args)
                    .current_dir(&dir),
            )
            .unwrap()
        };
        git(&["init", "-q"]);
        fs::write(dir.join("a"), "1").unwrap();
        fs::write(dir.join(".gitignore"), "/dist/\n").unwrap();
        git(&["add", "a", ".gitignore"]);
        git(&["commit", "-q", "-m", "one"]);

        let e = check_tree(&dir, "v1.0.0").unwrap_err();
        assert!(e.contains("doesn't exist"), "{e}");
        git(&["tag", "v1.0.0"]);
        let head = check_tree(&dir, "v1.0.0").unwrap();
        assert_eq!(head, git(&["rev-parse", "HEAD"]));

        // dist/COMMIT: what `make package` built from.
        let e = check_dist_commit(&dir, &head).unwrap_err();
        assert!(e.contains("missing"), "{e}");
        fs::create_dir_all(dir.join("dist")).unwrap();
        fs::write(dir.join("dist/COMMIT"), built_from(&dir).unwrap() + "\n").unwrap();
        assert_eq!(check_dist_commit(&dir, &head), Ok(()));

        fs::write(dir.join("a"), "2").unwrap();
        let e = check_tree(&dir, "v1.0.0").unwrap_err();
        assert!(e.contains("uncommitted"), "{e}");
        assert_eq!(built_from(&dir).unwrap(), format!("{head}-dirty"));
        fs::write(dir.join("dist/COMMIT"), built_from(&dir).unwrap()).unwrap();
        let e = check_dist_commit(&dir, &head).unwrap_err();
        assert!(e.contains("-dirty"), "{e}");

        // Packaged from the tag, then the tag moved on: refused.
        git(&["commit", "-q", "-am", "two"]);
        let e = check_tree(&dir, "v1.0.0").unwrap_err();
        assert!(e.contains("points at"), "{e}");
        fs::write(dir.join("dist/COMMIT"), &head).unwrap();
        let moved = git(&["rev-parse", "HEAD"]);
        let e = check_dist_commit(&dir, &moved).unwrap_err();
        assert!(e.contains("packaged from"), "{e}");
        let _ = fs::remove_dir_all(dir);
    }
}
