//! `make release`, `make package`, and `make publish` (SPEC §14.5, §15).
//!
//! - release: optimized binaries for this OS. Linux builds static musl for
//!   x86_64 and aarch64 (rust-lld, F22); macOS builds both Darwin targets,
//!   joins them with `lipo`, and re-signs ad hoc (F15); Windows builds
//!   `pawprint.exe` and `pawprintw.exe`. Each binary is checked (size budget,
//!   static linking, signature) and run where this machine can run it.
//! - package: release, then one archive per build plus a `sha256sum`-format
//!   sidecar into `dist/`, named exactly as SPEC §15.2 says.
//! - publish: upload `dist/*` and the installers to the GitHub release
//!   `v<version>` with `gh`, creating the release on first use.

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
            write_zip(&path, &files)?;
        } else {
            write_tar_gz(&path, &files)?;
        }
        let hash = write_sidecar(&path)?;
        let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        println!("  dist/{name}  {:.2} MB  sha256 {hash}", size as f64 / 1e6);
    }
    Ok(())
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
/// given modes.
pub fn write_tar_gz(path: &Path, files: &[Entry]) -> Result<(), String> {
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
        header.set_mtime(now());
        header.set_cksum();
        tar.append_data(&mut header, name, data.as_slice())
            .map_err(fail)?;
    }
    tar.into_inner().map_err(fail)?.finish().map_err(fail)?;
    Ok(())
}

/// A flat `.zip` (deflated).
pub fn write_zip(path: &Path, files: &[Entry]) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("cannot write {}: {e}", path.display());
    let mut zip = zip::ZipWriter::new(File::create(path).map_err(|e| fail(&e))?);
    for (name, src, mode) in files {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(*mode);
        zip.start_file(name.as_str(), options)
            .map_err(|e| fail(&e))?;
        zip.write_all(&read(src)?).map_err(|e| fail(&e))?;
    }
    zip.finish().map_err(|e| fail(&e))?;
    Ok(())
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

/// `make publish` (`DRY_RUN=1` prints the commands and runs nothing).
pub fn publish(dry_run: bool) -> Result<(), String> {
    let root = root();
    let (version, repo) = workspace_package(&root)?;
    let tag = format!("v{version}");
    let assets = publish_assets(&root, &version)?;
    let ready = check_tree(&root, &tag);

    let view = ["release", "view", &tag, "--repo", &repo]
        .map(String::from)
        .to_vec();
    let mut create: Vec<String> = ["release", "create", &tag, "--repo", &repo]
        .map(String::from)
        .to_vec();
    create.extend(
        [
            "--title",
            &format!("PAWPRINT {version}"),
            "--generate-notes",
            "--verify-tag",
        ]
        .map(String::from),
    );
    let mut upload: Vec<String> = ["release", "upload", &tag, "--repo", &repo, "--clobber"]
        .map(String::from)
        .to_vec();
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
    let gh = which("gh")
        .ok_or("make publish needs the GitHub CLI: https://cli.github.com, then `gh auth login`")?;
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
        println!("creating release {tag}");
        run(Command::new(&gh).args(&create).current_dir(&root))?;
    }
    run(Command::new(&gh).args(&upload).current_dir(&root))?;
    println!("published: https://github.com/{repo}/releases/tag/{tag}");
    Ok(())
}

/// `dist/*` (all of this version) and the two installers, relative to the root.
fn publish_assets(root: &Path, version: &str) -> Result<Vec<String>, String> {
    let dist = root.join("dist");
    let mut files: Vec<String> = fs::read_dir(&dist)
        .map_err(|_| "dist/ is empty: run `make package` first".to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
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
    assets.extend(["installers/install.sh", "installers/install.ps1"].map(String::from));
    Ok(assets)
}

/// A release must be built from exactly the tagged commit: a clean tree, and
/// the tag on HEAD.
pub fn check_tree(root: &Path, tag: &str) -> Result<(), String> {
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
    Ok(())
}

/// Quote arguments for display, so a printed command can be pasted.
fn shell_line(args: &[String]) -> String {
    std::iter::once("gh".to_string())
        .chain(args.iter().map(|a| {
            if a.contains(|c: char| c.is_whitespace() || "\"'$".contains(c)) {
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
        write_tar_gz(&tgz, &files).unwrap();
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
        write_zip(&zip_path, &files).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
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
        git(&["add", "a"]);
        git(&["commit", "-q", "-m", "one"]);

        let e = check_tree(&dir, "v1.0.0").unwrap_err();
        assert!(e.contains("doesn't exist"), "{e}");
        git(&["tag", "v1.0.0"]);
        assert_eq!(check_tree(&dir, "v1.0.0"), Ok(()));

        fs::write(dir.join("a"), "2").unwrap();
        let e = check_tree(&dir, "v1.0.0").unwrap_err();
        assert!(e.contains("uncommitted"), "{e}");
        git(&["commit", "-q", "-am", "two"]);
        let e = check_tree(&dir, "v1.0.0").unwrap_err();
        assert!(e.contains("points at"), "{e}");
        let _ = fs::remove_dir_all(dir);
    }
}
