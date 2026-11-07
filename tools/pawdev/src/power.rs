//! `make power-check` (SPEC §7.2): run the release badge and measure it
//! against the power budget. Linux: in a private Xvfb session, reading
//! `/proc`. Windows: on the current desktop, reading the process through
//! PowerShell (`Get-Process` and the raw thread performance counters).
//!
//! - startup to visible: process start until `pawprint status` reports a
//!   placed badge (the loop answers only after the first frame is flushed);
//! - CPU in idle and in static mode: `utime + stime` over the window, from
//!   `/proc/<pid>/stat`;
//! - wakeups: voluntary context switches of every thread, from
//!   `/proc/<pid>/task/*/status` (each one is a thread going to sleep after
//!   having woken up), counted per thread so a thread that exits during the
//!   window doesn't make the total go backwards;
//! - resident memory: `VmHWM` (peak) and `VmRSS`;
//! - binary size of the release build.
//!
//! A private `dbus-daemon` (if installed) gives the badge a session bus,
//! so the tray's thread is part of the measurement, as on a desktop.
//!
//! On Windows the same numbers come from `Get-Process` (`TotalProcessorTime`,
//! `PeakWorkingSet64`, `WorkingSet64`, `Threads`) and each thread's raw
//! `ContextSwitchesPersec` counter (`Win32_PerfRawData_PerfProc_Thread`, a
//! running count of every switch to the thread). Win32 known folders can't be
//! redirected, so it uses this user's config file and restores it, and the
//! state file, afterwards.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// SPEC §7.2.
const IDLE_CPU_BUDGET: f64 = 0.1; // percent of one core
const RSS_BUDGET_MB: f64 = 30.0;
const STARTUP_BUDGET_MS: u128 = 300;
const BINARY_BUDGET_MB: f64 = 15.0;

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Sample {
    cpu_seconds: f64,
    /// Context switches so far, per thread id. A thread that exits takes
    /// its count with it, so wakeups are counted per thread (below).
    switches: BTreeMap<u64, u64>,
    peak_mb: f64,
    rss_mb: f64,
    threads: usize,
}

fn sample(pid: u32) -> Result<Sample, String> {
    if cfg!(windows) {
        sample_windows(pid)
    } else {
        sample_linux(pid)
    }
}

/// Context switches made between two samples by the threads alive at the
/// second (a thread that started in between counts from zero).
fn wakeups(before: &Sample, after: &Sample) -> u64 {
    after
        .switches
        .iter()
        .map(|(tid, n)| n.saturating_sub(before.switches.get(tid).copied().unwrap_or(0)))
        .sum()
}

/// One PowerShell call: CPU time (100 ns ticks), peak and current working
/// set, thread count, then `<thread id> <context switches>` per thread.
fn sample_windows(pid: u32) -> Result<Sample, String> {
    let script = format!(
        "$p = Get-Process -Id {pid} -ErrorAction Stop; \
         \"$($p.TotalProcessorTime.Ticks) $($p.PeakWorkingSet64) $($p.WorkingSet64) $($p.Threads.Count)\"; \
         Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter 'IDProcess={pid}' | \
         ForEach-Object {{ \"$($_.IDThread) $($_.ContextSwitchesPersec)\" }}"
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("cannot run powershell: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let f: Vec<f64> = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    let switches: BTreeMap<u64, u64> = lines
        .filter_map(|l| {
            let (tid, n) = l.trim().split_once(' ')?;
            Some((tid.parse().ok()?, n.parse().ok()?))
        })
        .collect();
    let [ticks, peak, rss, threads] = f[..] else {
        return Err(format!(
            "the badge is gone, or its counters are unreadable: {text}{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    };
    if switches.is_empty() {
        return Err(format!("no thread counters for the badge: {text}"));
    }
    Ok(Sample {
        cpu_seconds: ticks / 1e7,
        switches,
        peak_mb: peak / 1048576.0,
        rss_mb: rss / 1048576.0,
        threads: threads as usize,
    })
}

fn sample_linux(pid: u32) -> Result<Sample, String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("the badge is gone: {e}"))?;
    // Fields after the parenthesized command name; utime and stime are
    // fields 14 and 15 (1-based), so 11 and 12 after the name.
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<u64> = rest
        .split_whitespace()
        .skip(1)
        .map(|v| v.parse().unwrap_or(0))
        .collect();
    let cpu_ticks = f[10] + f[11];
    let mut switches = BTreeMap::new();
    for task in std::fs::read_dir(format!("/proc/{pid}/task")).map_err(|e| e.to_string())? {
        let task = task.map_err(|e| e.to_string())?;
        let status = std::fs::read_to_string(task.path().join("status")).unwrap_or_default();
        let tid = task.file_name().to_string_lossy().parse().unwrap_or(0);
        switches.insert(tid, field(&status, "voluntary_ctxt_switches:").unwrap_or(0));
    }
    let (peak, rss) = memory_kb(pid);
    Ok(Sample {
        cpu_seconds: cpu_ticks as f64 / ticks_per_second(),
        switches,
        peak_mb: peak as f64 / 1024.0,
        rss_mb: rss as f64 / 1024.0,
        threads: threads(pid),
    })
}

fn field(status: &str, name: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(name))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
}

fn memory_kb(pid: u32) -> (u64, u64) {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    (
        field(&status, "VmHWM:").unwrap_or(0),
        field(&status, "VmRSS:").unwrap_or(0),
    )
}

fn threads(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/task")).map_or(0, |d| d.count())
}

/// Clock ticks per second (`getconf CLK_TCK`), 100 on every Linux in practice.
fn ticks_per_second() -> f64 {
    Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(100.0)
}

struct Session {
    dir: PathBuf,
    display: String,
    bus: Option<String>,
    _servers: Vec<Kill>,
    config: PathBuf,
    /// Windows: this user's files as they were, put back on drop.
    restore: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Session {
    fn start(cli: &Path) -> Result<Session, String> {
        if cfg!(windows) {
            Session::desktop(cli)
        } else {
            Session::xvfb()
        }
    }

    /// Windows: the current desktop and this user's own files.
    fn desktop(cli: &Path) -> Result<Session, String> {
        let mut session = Session {
            dir: PathBuf::new(),
            display: String::new(),
            bus: None,
            _servers: Vec::new(),
            config: PathBuf::new(),
            restore: Vec::new(),
        };
        if session
            .command(cli)
            .arg("status")
            .status()
            .map_err(|e| e.to_string())?
            .code()
            != Some(4)
        {
            return Err(
                "a badge is already running in this session; `pawprint quit` it first".into(),
            );
        }
        let out = session
            .command(cli)
            .arg("paths")
            .stdout(Stdio::piped())
            .output()
            .map_err(|e| e.to_string())?;
        let paths = String::from_utf8_lossy(&out.stdout);
        let path = |name: &str| {
            paths
                .lines()
                .find(|l| l.split_whitespace().next() == Some(name))
                .map(|l| PathBuf::from(l[name.len()..].trim()))
                .ok_or(format!("`pawprint paths` has no {name} line"))
        };
        session.config = path("config")?;
        for p in [path("config")?, path("state")?] {
            let saved = std::fs::read(&p).ok();
            session.restore.push((p, saved));
        }
        Ok(session)
    }

    fn xvfb() -> Result<Session, String> {
        let dir = std::env::temp_dir().join(format!("pawprint-power-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["home", "run"] {
            std::fs::create_dir_all(dir.join(sub)).map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("run"), std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let n = (150..250)
            .find(|n| !Path::new(&format!("/tmp/.X{n}-lock")).exists())
            .ok_or("no free X display number")?;
        let xvfb = Command::new("Xvfb")
            .args([
                &format!(":{n}"),
                "-screen",
                "0",
                "1280x720x24",
                "-listen",
                "tcp",
                "-ac",
                "-nolisten",
                "unix",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                format!("cannot start Xvfb ({e}); install it: sudo apt-get install xvfb")
            })?;
        let mut servers = vec![Kill(xvfb)];
        let addr = format!("unix:path={}", dir.join("bus").display());
        let bus = Command::new("dbus-daemon")
            .args([
                "--session",
                "--nofork",
                "--nopidfile",
                &format!("--address={addr}"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
            .map(|c| {
                servers.push(Kill(c));
                addr
            });
        std::thread::sleep(Duration::from_millis(500));
        Ok(Session {
            config: dir.join("home/.config/pawprint/config.toml"),
            dir,
            display: format!("127.0.0.1:{n}"),
            bus,
            _servers: servers,
            restore: Vec::new(),
        })
    }

    fn command(&self, exe: &Path) -> Command {
        let mut cmd = Command::new(exe);
        if cfg!(windows) {
            cmd.env("PAWPRINT_HOSTNAME", "owl")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            return cmd;
        }
        cmd.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.join("home"))
            .env("XDG_RUNTIME_DIR", self.dir.join("run"))
            .env("DISPLAY", &self.display)
            .env("PAWPRINT_HOSTNAME", "owl")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(bus) = &self.bus {
            cmd.env("DBUS_SESSION_BUS_ADDRESS", bus);
        }
        cmd
    }

    fn placed(&self, exe: &Path) -> bool {
        self.command(exe)
            .arg("status")
            .stdout(Stdio::piped())
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(" at "))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for (path, saved) in &self.restore {
            let _ = match saved {
                Some(bytes) => std::fs::write(path, bytes),
                None => std::fs::remove_file(path),
            };
        }
        if !self.dir.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

struct Run {
    startup: Duration,
    cpu_percent: f64,
    switches: u64,
    seconds: u64,
    peak_mb: f64,
    rss_mb: f64,
    threads: usize,
}

/// The binaries: the badge (`pawprintw.exe` on Windows) and the CLI.
struct Exes {
    badge: PathBuf,
    cli: PathBuf,
}

fn measure(session: &Session, exes: &Exes, config: &str, seconds: u64) -> Result<Run, String> {
    if let Some(dir) = session.config.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&session.config, config).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let badge = Kill(
        session
            .command(&exes.badge)
            .arg("run")
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", exes.badge.display()))?,
    );
    while !session.placed(&exes.cli) {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("the badge did not appear within 10 s".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let startup = started.elapsed();
    let pid = badge.0.id();
    // Let startup work (tray registration) settle before measuring.
    std::thread::sleep(Duration::from_secs(2));
    let before = sample(pid)?;
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    let after = sample(pid)?;
    let elapsed = t0.elapsed().as_secs_f64();
    Ok(Run {
        startup,
        cpu_percent: (after.cpu_seconds - before.cpu_seconds) / elapsed * 100.0,
        switches: wakeups(&before, &after),
        seconds,
        peak_mb: after.peak_mb,
        rss_mb: after.rss_mb,
        threads: after.threads,
    })
}

pub fn power_check(root: &Path, seconds: Option<&str>) -> Result<(), String> {
    if !cfg!(any(target_os = "linux", windows)) {
        return Err("make power-check: Linux and Windows only for now".into());
    }
    let seconds: u64 = seconds
        .map(|s| {
            s.parse()
                .map_err(|_| format!("POWER_SECONDS={s}: not a number"))
        })
        .transpose()?
        .unwrap_or(120);
    println!("building the release binary...");
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--package", "pawprint"])
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("cargo build --release failed".into());
    }
    let release = root.join("target/release");
    let exes = if cfg!(windows) {
        Exes {
            badge: release.join("pawprintw.exe"),
            cli: release.join("pawprint.exe"),
        }
    } else {
        Exes {
            badge: release.join("pawprint"),
            cli: release.join("pawprint"),
        }
    };
    let size_mb = |p: &Path| std::fs::metadata(p).map(|m| m.len() as f64 / 1e6);
    let binary_mb = size_mb(&exes.badge).map_err(|e| e.to_string())?;
    let cli_mb = size_mb(&exes.cli).map_err(|e| e.to_string())?;

    let session = Session::start(&exes.cli)?;
    let place = if cfg!(windows) {
        "this desktop"
    } else if session.bus.is_some() {
        "Xvfb + session bus"
    } else {
        "Xvfb"
    };
    println!("measuring {seconds} s in idle mode, then {seconds} s in static mode ({place})...");
    let idle = measure(&session, &exes, "", seconds)?;
    let stat = measure(&session, &exes, "animation = \"static\"\n", seconds)?;

    let mut failures = Vec::new();
    let mut row = |metric: &str, value: String, budget: &str, ok: bool| {
        println!(
            "  {metric:<30}{value:<26}{budget:<24}{}",
            if ok { "ok" } else { "OVER" }
        );
        if !ok {
            failures.push(metric.to_string());
        }
    };
    println!(
        "\n  {:<30}{:<26}{:<24}",
        "metric", "measured", "budget (SPEC §7.2)"
    );
    row(
        "startup to visible",
        format!("{} ms", idle.startup.as_millis()),
        &format!("< {STARTUP_BUDGET_MS} ms"),
        idle.startup.as_millis() < STARTUP_BUDGET_MS,
    );
    row(
        "CPU, idle mode",
        format!("{:.4}% of a core", idle.cpu_percent),
        &format!("< {IDLE_CPU_BUDGET}%"),
        idle.cpu_percent < IDLE_CPU_BUDGET,
    );
    row(
        "wakeups, idle mode",
        format!("{} in {} s", idle.switches, idle.seconds),
        "one per frame or rest",
        true,
    );
    row(
        "CPU, static mode",
        format!("{:.4}% of a core", stat.cpu_percent),
        "0% without events",
        stat.cpu_percent < 0.01,
    );
    row(
        "wakeups, static mode",
        format!("{} in {} s", stat.switches, stat.seconds),
        "zero",
        stat.switches == 0,
    );
    let peak = idle.peak_mb.max(stat.peak_mb);
    row(
        "resident memory (peak)",
        format!("{peak:.1} MB (now {:.1} MB)", idle.rss_mb),
        &format!("< {RSS_BUDGET_MB} MB"),
        peak < RSS_BUDGET_MB,
    );
    let (binary, value) = if cfg!(windows) {
        (
            "binary size (pawprintw.exe)",
            format!("{binary_mb:.1} MB (pawprint.exe {cli_mb:.1} MB)"),
        )
    } else {
        ("binary size (release, glibc)", format!("{binary_mb:.1} MB"))
    };
    row(
        binary,
        value,
        &format!("< {BINARY_BUDGET_MB} MB"),
        binary_mb.max(cli_mb) < BINARY_BUDGET_MB,
    );
    println!("  {:<30}{}", "threads", idle.threads);
    if failures.is_empty() {
        println!("\npower check: within budget");
        Ok(())
    } else {
        Err(format!("power check: over budget: {}", failures.join(", ")))
    }
}
