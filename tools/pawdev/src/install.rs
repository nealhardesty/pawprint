//! `make install`, `make uninstall`, and `make test-install` (SPEC §15.4–15.6).
//!
//! `make install` runs the real installer (`installers/install.sh`, or
//! `install.ps1` on Windows) with `--archive dist/<this machine's archive>`,
//! so a local install takes the same path as a downloaded one.
//!
//! `make test-install` runs each installer under every shell this machine
//! has (sh, dash, and bash; Windows PowerShell 5.1 and PowerShell 7) in a
//! throwaway home: a fresh install from the archive, an upgrade from a local
//! HTTP server laid out like GitHub releases (so "latest" resolves through
//! the same redirect), `pawprint update` from the installed binary against
//! a release that is newer (SPEC §15.8), a checksum mismatch that must
//! change nothing, an uninstall that keeps the config, and a purge.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use crate::release::{self, Build, sha256_hex, which};
use crate::root;

/// What to ask an installer for; spelled per installer by [`Shell::args`].
enum Opt<'a> {
    Archive(&'a Path),
    Version(&'a str),
    Prefix(&'a str),
    NoStart,
    NoAutostart,
    Uninstall,
    Purge,
}

/// An interpreter that runs one of the installers.
struct Shell {
    name: &'static str,
    exe: PathBuf,
}

impl Shell {
    fn powershell(&self) -> bool {
        matches!(self.name, "powershell" | "pwsh")
    }

    fn command(&self, root: &Path, opts: &[Opt]) -> Command {
        let mut cmd = Command::new(&self.exe);
        if self.powershell() {
            // A PSModulePath inherited from PowerShell 7 (a CI step's shell)
            // hides Windows PowerShell 5.1's own modules (Get-FileHash);
            // without it, each shell builds its default.
            cmd.env_remove("PSModulePath");
            cmd.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(root.join("installers").join("install.ps1"));
        } else {
            cmd.arg(root.join("installers").join("install.sh"));
        }
        cmd.args(self.args(opts));
        cmd
    }

    fn args(&self, opts: &[Opt]) -> Vec<OsString> {
        let ps = self.powershell();
        let flag = |sh: &str, pwsh: &str| OsString::from(if ps { pwsh } else { sh });
        let mut out = Vec::new();
        for opt in opts {
            match opt {
                Opt::Archive(p) => out.extend([flag("--archive", "-Archive"), p.into()]),
                Opt::Version(v) => out.extend([flag("--version", "-Version"), v.into()]),
                Opt::Prefix(p) => out.extend([flag("--prefix", "-Prefix"), p.into()]),
                Opt::NoStart => out.push(flag("--no-start", "-NoStart")),
                Opt::NoAutostart => out.push(flag("--no-autostart", "-NoAutostart")),
                Opt::Uninstall => out.push(flag("--uninstall", "-Uninstall")),
                Opt::Purge => out.push(flag("--purge", "-Purge")),
            }
        }
        out
    }
}

/// The shells this OS's installer must work under, those found on `PATH`.
/// The first is the one `make install` uses.
fn shells() -> Vec<Shell> {
    let names: &[&'static str] = if cfg!(windows) {
        &["powershell", "pwsh"]
    } else if cfg!(target_os = "macos") {
        &["sh", "bash"]
    } else {
        &["sh", "dash", "bash"]
    };
    names
        .iter()
        .filter_map(|&name| which(name).map(|exe| Shell { name, exe }))
        .collect()
}

fn default_shell() -> Result<Shell, String> {
    shells()
        .into_iter()
        .next()
        .ok_or_else(|| "no shell found to run the installer".to_string())
}

fn native_archive(root: &Path) -> Result<(Build, PathBuf), String> {
    let (version, _) = release::workspace_package(root)?;
    let build = release::native_build(root);
    let archive = root.join("dist").join(build.archive_name(&version));
    if !archive.is_file() {
        return Err(format!(
            "{} is missing: run `make package` first",
            archive.display()
        ));
    }
    Ok((build, archive))
}

/// `make install`.
pub fn install() -> Result<(), String> {
    let root = root();
    let (_, archive) = native_archive(&root)?;
    release::run(&mut default_shell()?.command(&root, &[Opt::Archive(&archive)]))
}

/// `make uninstall`.
pub fn uninstall() -> Result<(), String> {
    let root = root();
    release::run(&mut default_shell()?.command(&root, &[Opt::Uninstall]))
}

/// `make lint-installers`: install.sh must be shellcheck-clean as POSIX sh.
pub fn lint() -> Result<(), String> {
    let shellcheck = which("shellcheck").ok_or(
        "make lint-installers needs shellcheck (apt install shellcheck, brew install shellcheck)",
    )?;
    let script = root().join("installers").join("install.sh");
    release::run(
        Command::new(shellcheck)
            .args(["--shell=sh", "--severity=style"])
            .arg(&script),
    )?;
    println!("shellcheck: {} is clean", script.display());
    Ok(())
}

/// Where an installer puts things inside a throwaway home (SPEC §8.1).
struct Layout {
    home: PathBuf,
    bin_dir: PathBuf,
    /// Installed with `PAWPRINT_PREFIX`.
    alt_dir: PathBuf,
    config: PathBuf,
    state: Vec<PathBuf>,
    autostart: Option<PathBuf>,
}

impl Layout {
    fn new(home: PathBuf) -> Layout {
        let (bin_dir, config, state, autostart);
        if cfg!(windows) {
            bin_dir = home.join("Local").join("Programs").join("pawprint");
            config = home.join("Roaming").join("pawprint");
            state = vec![home.join("Local").join("pawprint")];
            autostart = None; // HKCU Run, checked through `pawprint autostart status`
        } else if cfg!(target_os = "macos") {
            bin_dir = home.join(".local/bin");
            config = home.join("Library/Application Support/pawprint");
            state = vec![home.join("Library/Logs/pawprint")];
            autostart = None; // checked through `pawprint autostart status`
        } else {
            bin_dir = home.join(".local/bin");
            config = home.join(".config/pawprint");
            state = vec![home.join(".local/state/pawprint")];
            autostart = Some(home.join(".config/autostart/pawprint.desktop"));
        }
        Layout {
            alt_dir: home.join("alt prefix"),
            home,
            bin_dir,
            config,
            state,
            autostart,
        }
    }

    fn exe(dir: &Path) -> PathBuf {
        dir.join(format!("pawprint{}", std::env::consts::EXE_SUFFIX))
    }

    /// The environment of an installer run: the throwaway home, and on Unix a
    /// private IPC directory, so a badge running on this desktop is left alone.
    fn env(&self, cmd: &mut Command) {
        for var in [
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_DATA_HOME",
            "PAWPRINT_VERSION",
            "PAWPRINT_PREFIX",
        ] {
            cmd.env_remove(var);
        }
        if cfg!(windows) {
            cmd.env("LOCALAPPDATA", self.home.join("Local"))
                .env("APPDATA", self.home.join("Roaming"));
        } else {
            cmd.env("HOME", &self.home)
                .env("XDG_RUNTIME_DIR", self.home.join("run"))
                .env("TMPDIR", self.home.join("tmp"));
        }
    }
}

struct Run {
    ok: bool,
    text: String,
}

/// One installer test: a shell, a throwaway home, and the release it serves.
struct Case<'a> {
    root: &'a Path,
    shell: &'a Shell,
    layout: Layout,
    version: &'a str,
    archive: &'a Path,
    server: &'a str,
    /// The binary the archive holds (target/<triple>/release/pawprint).
    probe: &'a Path,
    log: Vec<String>,
}

impl Case<'_> {
    fn installer(&self, opts: &[Opt], base: &str, extra_env: &[(&str, &Path)]) -> Run {
        self.installer_in(None, opts, base, extra_env)
    }

    /// The installer run from `cwd` (as a shell would: `PWD` set too).
    fn installer_in(
        &self,
        cwd: Option<&Path>,
        opts: &[Opt],
        base: &str,
        extra_env: &[(&str, &Path)],
    ) -> Run {
        let mut cmd = self.shell.command(self.root, opts);
        self.layout.env(&mut cmd);
        if let Some(dir) = cwd {
            cmd.current_dir(dir).env("PWD", dir);
        }
        cmd.env(
            "PAWPRINT_DOWNLOAD_BASE",
            format!("{}/{base}/releases", self.server),
        );
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        self.capture(cmd)
    }

    /// Run and collect stdout and stderr through a file, not a pipe: on
    /// Windows the badge an installer starts inherits PowerShell's own
    /// inheritable handles (FW8, one level up), so a pipe would stay open
    /// until the badge quits.
    fn capture(&self, mut cmd: Command) -> Run {
        static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let log = self.layout.home.join(format!("output-{n}.log"));
        let run = (|| {
            let file = std::fs::File::create(&log)?;
            let status = cmd
                .stdin(std::process::Stdio::null())
                .stdout(file.try_clone()?)
                .stderr(file)
                .status()?;
            Ok::<_, std::io::Error>(Run {
                ok: status.success(),
                text: String::from_utf8_lossy(&std::fs::read(&log)?).into_owned(),
            })
        })();
        // A badge still holding it (Windows) only keeps it from being deleted.
        let _ = std::fs::remove_file(&log);
        run.unwrap_or_else(|e| Run {
            ok: false,
            text: format!("cannot run {}: {e}", cmd.get_program().to_string_lossy()),
        })
    }

    /// Run the installed (or the probe) binary in the throwaway home.
    fn pawprint(&self, exe: &Path, args: &[&str]) -> Run {
        let mut cmd = Command::new(exe);
        cmd.args(args);
        self.layout.env(&mut cmd);
        self.capture(cmd)
    }

    /// `pawprint update` from the installed binary, against the release
    /// repository `base` on the test server.
    fn update(&self, exe: &Path, args: &[&str], base: &str) -> Run {
        let mut cmd = Command::new(exe);
        cmd.arg("update").args(args);
        self.layout.env(&mut cmd);
        cmd.env(
            "PAWPRINT_DOWNLOAD_BASE",
            format!("{}/{base}/releases", self.server),
        );
        self.capture(cmd)
    }

    fn running(&self) -> bool {
        self.pawprint(self.probe, &["status"]).ok
    }

    fn check(&mut self, ok: bool, what: &str, run: Option<&Run>) -> Result<(), String> {
        if ok {
            self.log.push(format!("    ok: {what}"));
            Ok(())
        } else {
            let output = run.map_or(String::new(), |r| {
                format!("\n--- installer output ---\n{}", r.text)
            });
            Err(format!(
                "{} ({}): {what}{output}",
                self.shell.name,
                self.shell.exe.display()
            ))
        }
    }

    fn note(&mut self, what: String) {
        self.log.push(format!("    note: {what}"));
    }

    fn installed_ok(&mut self, dir: &Path, run: &Run) -> Result<(), String> {
        let exe = Layout::exe(dir);
        self.check(
            exe.is_file(),
            &format!("{} exists", exe.display()),
            Some(run),
        )?;
        if cfg!(windows) {
            let w = dir.join("pawprintw.exe");
            self.check(w.is_file(), "pawprintw.exe installed next to it", Some(run))?;
        }
        let v = self.pawprint(&exe, &["--version"]);
        let want = format!("pawprint {} (", self.version);
        self.check(
            v.ok && v.text.starts_with(&want),
            &format!("pawprint --version: {}", v.text.trim()),
            Some(run),
        )?;
        let same = std::fs::read(&exe).ok().map(|d| sha256_hex(&d))
            == std::fs::read(self.probe).ok().map(|d| sha256_hex(&d));
        self.check(same, "the installed binary is the packaged one", Some(run))
    }

    fn autostart_ok(&mut self, exe: &Path, run: &Run) -> Result<(), String> {
        if let Some(entry) = self.layout.autostart.clone() {
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            return self.check(
                text.contains(&format!("Exec={} run", exe.display())),
                &format!(
                    "autostart entry {} runs the installed binary",
                    entry.display()
                ),
                Some(run),
            );
        }
        // Windows (HKCU Run) and macOS (LaunchAgent): ask the binary.
        let status = self.pawprint(exe, &["autostart", "status"]);
        let on = status.ok && status.text.starts_with("on:") && status.text.contains("pawprint");
        self.check(
            on,
            &format!("autostart status: {}", status.text.trim()),
            Some(run),
        )
    }

    /// No autostart entry: the XDG file on Linux; on Windows and macOS, what
    /// the packaged binary reports (the entry is per user, not per install).
    fn autostart_off(&mut self, run: &Run, what: &str) -> Result<(), String> {
        if let Some(entry) = self.layout.autostart.clone() {
            return self.check(!entry.exists(), what, Some(run));
        }
        let status = self.pawprint(self.probe, &["autostart", "status"]);
        self.check(status.ok && status.text.starts_with("off"), what, Some(run))
    }

    #[cfg(windows)]
    fn user_path_has(&self, dir: &Path) -> bool {
        // `reg query` prints the raw value on one line:
        //     Path    REG_EXPAND_SZ    C:\a;C:\b
        let out = Command::new("reg")
            .args(["query", "HKCU\\Environment", "/v", "Path"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        out.lines()
            .filter_map(|l| l.split_once("REG_").map(|(_, rest)| rest))
            .filter_map(|rest| rest.split_once("    ").map(|(_, value)| value.trim()))
            .flat_map(|value| value.split(';'))
            .any(|p| Path::new(p) == dir)
    }

    fn scenario(&mut self) -> Result<(), String> {
        let l = &self.layout;
        let (bin_dir, alt_dir, config) = (l.bin_dir.clone(), l.alt_dir.clone(), l.config.clone());
        let exe = Layout::exe(&bin_dir);
        let version = self.version.to_string();

        // 1. Fresh install from the local archive (starts the badge where
        //    there is a desktop; warns and carries on where there isn't).
        let run = self.installer(&[Opt::Archive(self.archive)], "good", &[]);
        self.check(run.ok, "fresh install from --archive succeeds", Some(&run))?;
        self.installed_ok(&bin_dir, &run)?;
        self.autostart_ok(&exe, &run)?;
        self.check(
            run.text.contains("PATH"),
            "prints a PATH hint (the prefix is not on PATH)",
            Some(&run),
        )?;
        self.check(
            run.text.contains(
                &self
                    .pawprint(&exe, &["explain"])
                    .text
                    .lines()
                    .next()
                    .unwrap_or("?")
                    .to_string(),
            ),
            "prints `pawprint explain`",
            Some(&run),
        )?;
        #[cfg(windows)]
        {
            let on = self.user_path_has(&bin_dir);
            self.check(
                on,
                "adds the install directory to the user PATH",
                Some(&run),
            )?;
        }
        let running = self.running();
        self.note(if running {
            "the badge is running (this machine has a desktop session)".into()
        } else {
            "the badge did not start (no desktop session); upgrade restarts are not exercised"
                .into()
        });
        std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
        std::fs::write(config.join("config.toml"), "# kept across upgrades\n")
            .map_err(|e| e.to_string())?;

        // 2. Upgrade in place from the "latest" release over HTTP.
        let run = self.installer(&[], "good", &[]);
        self.check(
            run.ok,
            "upgrade from the latest release over HTTP succeeds",
            Some(&run),
        )?;
        self.check(
            run.text.contains("Upgrading"),
            "says it is upgrading",
            Some(&run),
        )?;
        self.installed_ok(&bin_dir, &run)?;
        if running {
            let again = self.running();
            self.check(again, "the badge was stopped and started again", Some(&run))?;
        }
        self.check(
            config.join("config.toml").is_file(),
            "config kept on upgrade",
            Some(&run),
        )?;

        // 2b. `pawprint update` (SPEC §15.8): nothing to do while this is the
        //     latest release; a newer one is found, then installed in
        //     place with the installer the binary carries, the badge
        //     restarted, and autostart left as it was.
        let run = self.update(&exe, &["--check"], "good");
        self.check(
            run.ok && run.text.contains("is the latest release"),
            "update --check: this is the latest release",
            Some(&run),
        )?;
        let run = self.update(&exe, &[], "good");
        self.check(
            run.ok && run.text.contains("is the latest release") && !run.text.contains("Upgrading"),
            "update: nothing to install",
            Some(&run),
        )?;
        let run = self.update(&exe, &["--check"], "next");
        self.check(
            run.ok && run.text.contains(&format!("PAWPRINT {NEXT} is available")),
            "update --check: finds the newer release",
            Some(&run),
        )?;
        let before = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
        let run = self.update(&exe, &[], "next");
        self.check(run.ok, "update installs the newer release", Some(&run))?;
        self.check(
            run.text.contains("Upgrading") && run.text.contains("Checksum OK"),
            "through the installer: verified, then upgraded",
            Some(&run),
        )?;
        let after = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
        self.check(before != after, "the binary was replaced", Some(&run))?;
        self.installed_ok(&bin_dir, &run)?;
        self.autostart_ok(&exe, &run)?;
        if running {
            let again = self.running();
            self.check(again, "the badge was restarted", Some(&run))?;
        } else {
            let off = !self.running();
            self.check(off, "no badge was started (none was running)", Some(&run))?;
        }
        self.check(
            config.join("config.toml").is_file(),
            "config kept on update",
            Some(&run),
        )?;

        // 3. A checksum mismatch changes nothing.
        let before = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
        let run = self.installer(&[Opt::Version(&version)], "bad", &[]);
        self.check(!run.ok, "a checksum mismatch fails", Some(&run))?;
        self.check(
            run.text.to_lowercase().contains("checksum"),
            "and says why",
            Some(&run),
        )?;
        let after = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
        self.check(
            before.is_some() && before == after,
            "the installed binary is untouched",
            Some(&run),
        )?;
        self.installed_ok(&bin_dir, &run)?;
        if running {
            let still = self.running();
            self.check(still, "the running badge was left alone", Some(&run))?;
        }

        // 4. Uninstall keeps the config.
        let run = self.installer(&[Opt::Uninstall], "good", &[]);
        self.check(run.ok, "--uninstall succeeds", Some(&run))?;
        self.check(!exe.exists(), "the binary is gone", Some(&run))?;
        if cfg!(windows) {
            let w = bin_dir.join("pawprintw.exe");
            self.check(!w.exists(), "pawprintw.exe is gone", Some(&run))?;
        }
        self.autostart_off(&run, "the autostart entry is gone")?;
        let stopped = !self.running();
        self.check(stopped, "no badge is running", Some(&run))?;
        self.check(
            config.join("config.toml").is_file(),
            "config kept on uninstall",
            Some(&run),
        )?;
        #[cfg(windows)]
        {
            let gone = !self.user_path_has(&bin_dir);
            self.check(gone, "the install directory left the user PATH", Some(&run))?;
        }

        // 5. Reinstall into PAWPRINT_PREFIX at PAWPRINT_VERSION, then purge.
        let env: [(&str, &Path); 2] = [
            ("PAWPRINT_PREFIX", &alt_dir),
            ("PAWPRINT_VERSION", Path::new(&version)),
        ];
        let run = self.installer(&[Opt::NoStart, Opt::NoAutostart], "good", &env);
        self.check(
            run.ok,
            "install with PAWPRINT_PREFIX and PAWPRINT_VERSION succeeds",
            Some(&run),
        )?;
        self.installed_ok(&alt_dir, &run)?;
        self.autostart_off(&run, "--no-autostart adds no autostart entry")?;
        for dir in self.layout.state.clone() {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("state.toml"), "").map_err(|e| e.to_string())?;
        }
        // Linux: relative XDG_CONFIG_HOME and XDG_STATE_HOME are invalid (the
        // XDG spec says to ignore them), so purge must not follow them.
        let home = self.layout.home.clone();
        let (rel_config, rel_state) = (home.join("rel-config"), home.join("rel-state"));
        let mut purge_env = env.to_vec();
        if cfg!(target_os = "linux") {
            for dir in [&rel_config, &rel_state] {
                std::fs::create_dir_all(dir.join("pawprint")).map_err(|e| e.to_string())?;
            }
            purge_env.push(("XDG_CONFIG_HOME", Path::new("rel-config")));
            purge_env.push(("XDG_STATE_HOME", Path::new("rel-state")));
        }
        let run = self.installer_in(
            Some(&home),
            &[Opt::Uninstall, Opt::Purge],
            "good",
            &purge_env,
        );
        self.check(run.ok, "--uninstall --purge succeeds", Some(&run))?;
        if cfg!(target_os = "linux") {
            let kept = rel_config.join("pawprint").is_dir() && rel_state.join("pawprint").is_dir();
            self.check(
                kept,
                "--purge ignores relative XDG_CONFIG_HOME and XDG_STATE_HOME",
                Some(&run),
            )?;
        }
        self.check(
            !Layout::exe(&alt_dir).exists(),
            "the binary is gone",
            Some(&run),
        )?;
        self.check(!config.exists(), "config deleted", Some(&run))?;
        for dir in self.layout.state.clone() {
            self.check(
                !dir.exists(),
                &format!("{} deleted", dir.display()),
                Some(&run),
            )?;
        }

        // 6. Relative paths resolve against the working directory: the
        //    archive (on Unix in a directory with a backslash in its name,
        //    which GNU sha256sum escapes in its output), and the prefix (on
        //    Windows with a trailing backslash, which is dropped).
        let sub = if cfg!(windows) {
            "archive dir"
        } else {
            "back\\slash"
        };
        std::fs::create_dir_all(home.join(sub)).map_err(|e| e.to_string())?;
        let name = self
            .archive
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for file in [name.clone(), format!("{name}.sha256")] {
            std::fs::copy(
                self.archive.with_file_name(&file),
                home.join(sub).join(&file),
            )
            .map_err(|e| format!("{file}: {e}"))?;
        }
        let rel_archive = Path::new(sub).join(&name);
        let rel_dir = home.join("rel bin");
        let prefix = if cfg!(windows) {
            "rel bin\\"
        } else {
            "rel bin"
        };
        let run = self.installer_in(
            Some(&home),
            &[
                Opt::Archive(&rel_archive),
                Opt::Prefix(prefix),
                Opt::NoStart,
                Opt::NoAutostart,
            ],
            "good",
            &[],
        );
        self.check(
            run.ok && run.text.contains("Checksum OK"),
            "install from a relative --archive (checksum checked) into a relative --prefix",
            Some(&run),
        )?;
        self.installed_ok(&rel_dir, &run)?;
        let shown = if cfg!(windows) {
            rel_dir.clone()
        } else {
            Layout::exe(&rel_dir)
        };
        let absolute = run
            .text
            .lines()
            .any(|l| l.trim_end().ends_with(&format!(" to {}", shown.display())));
        self.check(
            absolute,
            &format!("says it installed to {}", shown.display()),
            Some(&run),
        )?;

        // 7. Uninstall it through `~` (Unix) or the relative prefix.
        let prefix = if cfg!(windows) {
            "rel bin"
        } else {
            "~/rel bin"
        };
        let run = self.installer_in(
            Some(&home),
            &[Opt::Uninstall, Opt::Prefix(prefix)],
            "good",
            &[],
        );
        self.check(
            run.ok && !Layout::exe(&rel_dir).exists(),
            &format!("--uninstall --prefix '{prefix}' removes it"),
            Some(&run),
        )?;
        #[cfg(windows)]
        {
            let gone = !self.user_path_has(&rel_dir);
            self.check(gone, "the relative prefix left the user PATH", Some(&run))?;
        }
        Ok(())
    }
}

/// The version of the test server's `next` release: the packaged build
/// again, published as something newer, for `pawprint update` to find.
const NEXT: &str = "99.0.0";

/// macOS and Windows: the autostart entry is per user, not per home, so the
/// test would replace a developer's real entry and then delete it. Refuse
/// when it is on, unless in CI.
fn refuse_real_autostart(ci: bool, status: &str) -> Result<(), String> {
    if ci || !status.trim_start().starts_with("on") {
        return Ok(());
    }
    Err(format!(
        "make test-install: autostart is on for this user ({}).\n\
         On this OS the test would replace that entry and then remove it. Run \
         `pawprint autostart off` first (and `pawprint autostart on` afterwards), \
         or run the test where PAWPRINT isn't installed. In CI (CI set) this check is skipped.",
        status.trim()
    ))
}

/// `make test-install` (after `make package`).
pub fn test_install() -> Result<(), String> {
    let root = root();
    let (version, _) = release::workspace_package(&root)?;
    let (build, archive) = native_archive(&root)?;
    let name = archive.file_name().unwrap().to_string_lossy().into_owned();
    let data = std::fs::read(&archive).map_err(|e| e.to_string())?;
    let sidecar = std::fs::read(archive.with_file_name(format!("{name}.sha256")))
        .map_err(|e| format!("{name}.sha256: {e}"))?;
    let wrong = format!("{}  {name}\n", sha256_hex(b"not the archive"));

    // Three release repositories: `good` as published, `bad` whose
    // checksum is wrong, and `next`, whose latest release is newer.
    let next_name = build.archive_name(NEXT);
    let mut files = HashMap::new();
    let mut tags = HashMap::new();
    for (repo, release, name, sum) in [
        ("good", version.as_str(), &name, sidecar.clone()),
        ("bad", version.as_str(), &name, wrong.into_bytes()),
        ("next", NEXT, &next_name, sidecar),
    ] {
        let dir = format!("/{repo}/releases/download/v{release}");
        files.insert(format!("{dir}/{name}"), data.clone());
        files.insert(format!("{dir}/{name}.sha256"), sum);
        tags.insert(repo.to_string(), format!("v{release}"));
    }
    if !cfg!(target_os = "linux") {
        let ci = std::env::var_os("CI").is_some_and(|v| !v.is_empty());
        let status = release::output(Command::new(&build.bins[0]).args(["autostart", "status"]))
            .unwrap_or_default();
        refuse_real_autostart(ci, &status)?;
    }
    let server = serve(files, tags)?;
    println!("serving the release at {server}");

    let shells = shells();
    if shells.is_empty() {
        return Err("no shell found to run the installer".into());
    }
    let mut results = Vec::new();
    let mut failures = 0;
    for shell in &shells {
        // Under /tmp on Unix: macOS's $TMPDIR is long, and the badge's IPC
        // socket inside the home must fit in 104 bytes.
        let tmp = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let home = tmp.join(format!("pawprint-{}-{}", shell.name, std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for sub in ["run", "tmp"] {
            std::fs::create_dir_all(home.join(sub)).map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(home.join("run"), std::fs::Permissions::from_mode(0o700));
        }
        // Windows: the temp dir can be an 8.3 short path (C:\Users\RUNNER~1),
        // which PowerShell reports in long form; compare long forms only.
        #[cfg(windows)]
        let home = std::fs::canonicalize(&home)
            .ok()
            .and_then(|p| {
                p.to_str()
                    .map(|s| PathBuf::from(s.trim_start_matches(r"\\?\")))
            })
            .unwrap_or(home);
        println!(
            "\n{} ({}), home {}",
            shell.name,
            shell.exe.display(),
            home.display()
        );
        let mut case = Case {
            root: &root,
            shell,
            layout: Layout::new(home.clone()),
            version: &version,
            archive: &archive,
            server: &server,
            probe: &build.bins[0],
            log: Vec::new(),
        };
        let result = case.scenario();
        for line in &case.log {
            println!("{line}");
        }
        match result {
            Ok(()) => {
                results.push(format!("  {:<12} pass", shell.name));
                let _ = std::fs::remove_dir_all(&home);
            }
            Err(e) => {
                // Leave nothing running or registered behind.
                let _ = case.installer(&[Opt::Uninstall], "good", &[]);
                let alt: [(&str, &Path); 1] = [("PAWPRINT_PREFIX", &case.layout.alt_dir)];
                let _ = case.installer(&[Opt::Uninstall], "good", &alt);
                let rel: [(&str, &Path); 1] = [("PAWPRINT_PREFIX", &home.join("rel bin"))];
                let _ = case.installer(&[Opt::Uninstall], "good", &rel);
                println!("    FAIL: {e}");
                results.push(format!("  {:<12} FAIL", shell.name));
                failures += 1;
            }
        }
    }
    println!("\ninstaller tests:\n{}", results.join("\n"));
    match failures {
        0 => Ok(()),
        n => Err(format!(
            "installer tests: {n} shell(s) failed (details above)"
        )),
    }
}

/// A tiny HTTP server laid out like GitHub releases: `/<repo>/releases/latest`
/// redirects to `/<repo>/releases/tag/<tag>` (`tags` has each repository's
/// latest), and files are served from the map. Returns its base URL.
fn serve(files: HashMap<String, Vec<u8>>, tags: HashMap<String, String>) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let base = format!(
        "http://{}",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let state = Arc::new((files, tags, base.clone()));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            std::thread::spawn(move || {
                let _ = respond(stream, &state.0, &state.1, &state.2);
            });
        }
    });
    Ok(base)
}

fn respond(
    mut stream: TcpStream,
    files: &HashMap<String, Vec<u8>>,
    tags: &HashMap<String, String>,
    base: &str,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let mut parts = request.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let latest = path
        .strip_suffix("/releases/latest")
        .and_then(|repo| Some((repo, tags.get(repo.trim_start_matches('/'))?)));
    let (status, extra, body): (&str, String, &[u8]) = if let Some((repo, tag)) = latest {
        (
            "302 Found",
            format!("Location: {base}{repo}/releases/tag/{tag}\r\n"),
            b"",
        )
    } else if path.contains("/releases/tag/") {
        ("200 OK", String::new(), b"release page")
    } else if let Some(data) = files.get(path) {
        ("200 OK", String::new(), data)
    } else {
        ("404 Not Found", String::new(), b"not found")
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(body)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_flags_per_shell() {
        let sh = Shell {
            name: "dash",
            exe: "dash".into(),
        };
        let ps = Shell {
            name: "pwsh",
            exe: "pwsh".into(),
        };
        let opts = [
            Opt::Version("1.2.0"),
            Opt::NoStart,
            Opt::Uninstall,
            Opt::Purge,
        ];
        assert_eq!(
            sh.args(&opts),
            ["--version", "1.2.0", "--no-start", "--uninstall", "--purge"]
        );
        assert_eq!(
            ps.args(&opts),
            ["-Version", "1.2.0", "-NoStart", "-Uninstall", "-Purge"]
        );
        assert!(
            Shell {
                name: "powershell",
                exe: "x".into()
            }
            .powershell()
        );
        assert!(
            !Shell {
                name: "bash",
                exe: "x".into()
            }
            .powershell()
        );
    }

    #[test]
    fn test_install_refuses_a_real_autostart_entry_outside_ci() {
        let on = "on: HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\pawprint runs pawprintw.exe run";
        let off = "off (`pawprint autostart on` starts the badge at login)";
        let e = refuse_real_autostart(false, on).unwrap_err();
        assert!(e.contains("autostart off"), "{e}");
        assert_eq!(refuse_real_autostart(true, on), Ok(()));
        assert_eq!(refuse_real_autostart(false, off), Ok(()));
        assert_eq!(refuse_real_autostart(false, ""), Ok(()));
    }

    #[test]
    fn server_redirects_latest_and_serves_files() {
        let mut files = HashMap::new();
        files.insert(
            "/r/releases/download/v1.0.0/a".to_string(),
            b"hello".to_vec(),
        );
        let tags = HashMap::from([("r".to_string(), "v1.0.0".to_string())]);
        let base = serve(files, tags).unwrap();
        let get = |path: &str| {
            let mut s = TcpStream::connect(base.trim_start_matches("http://")).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            std::io::Read::read_to_string(&mut s, &mut out).unwrap();
            out
        };
        let latest = get("/r/releases/latest");
        assert!(latest.starts_with("HTTP/1.1 302"), "{latest}");
        assert!(latest.contains(&format!("Location: {base}/r/releases/tag/v1.0.0\r\n")));
        assert!(get("/r/releases/download/v1.0.0/a").ends_with("\r\n\r\nhello"));
        assert!(get("/nope").starts_with("HTTP/1.1 404"));
        assert!(get("/other/releases/latest").starts_with("HTTP/1.1 404"));
    }
}
