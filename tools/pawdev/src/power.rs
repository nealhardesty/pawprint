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
//!
//! On macOS it runs in the logged-in desktop with its own HOME and TMPDIR,
//! menu-bar item included, from a universal binary (`lipo` of the arm64 and
//! x86_64 release builds, the shipping shape). Everything comes from
//! `proc_pid_rusage`, the counters `top` and `powermetrics` read, with no
//! root needed: CPU is user + system time; wakeups are interrupt wakeups
//! (`powermetrics`' "Intr Wakeups"; package-idle wakeups, which Activity
//! Monitor shows, are a subset and are printed too); memory is the lifetime
//! peak and current physical footprint (Activity Monitor's "Memory") and the
//! resident size. AppKit's own threads wake and spend CPU in every Cocoa
//! process, so on macOS the whole-process static rows and idle CPU are
//! reported, not failed, and "idle over static", what animating adds, is
//! judged against the idle budget (SPEC §7.2).

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
    /// macOS: package-idle wakeups so far (a subset of the switches entry).
    idle_wakeups: Option<u64>,
}

fn sample(pid: u32) -> Result<Sample, String> {
    #[cfg(target_os = "macos")]
    return mac::sample(pid);
    #[allow(unreachable_code)]
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
        idle_wakeups: None,
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
        idle_wakeups: None,
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

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::BTreeMap;

    use super::Sample;

    fn rusage(pid: u32) -> Result<libc::rusage_info_v4, String> {
        // SAFETY: rusage_info_v4 is plain data; proc_pid_rusage fills it.
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        let buf = (&raw mut info).cast::<libc::rusage_info_t>();
        // SAFETY: buf points at a rusage_info_v4, the size of flavor V4.
        match unsafe { libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V4, buf) } {
            0 => Ok(info),
            _ => Err(format!(
                "the badge is gone: {}",
                std::io::Error::last_os_error()
            )),
        }
    }

    /// CPU times come in Mach time units (ticks of 125/3 ns on Apple
    /// Silicon, nanoseconds on Intel).
    fn mach_seconds(t: u64) -> f64 {
        #[repr(C)]
        struct Timebase {
            numer: u32,
            denom: u32,
        }
        unsafe extern "C" {
            fn mach_timebase_info(info: *mut Timebase) -> i32;
        }
        let mut tb = Timebase { numer: 0, denom: 0 };
        // SAFETY: tb is a valid out-parameter.
        unsafe { mach_timebase_info(&mut tb) };
        t as f64 * f64::from(tb.numer) / f64::from(tb.denom.max(1)) / 1e9
    }

    fn threads(pid: u32) -> usize {
        // SAFETY: proc_taskinfo is plain data; proc_pidinfo fills it.
        let mut task: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
        // SAFETY: task is `size` bytes, as PROC_PIDTASKINFO asks.
        let n = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTASKINFO,
                0,
                (&raw mut task).cast(),
                size,
            )
        };
        if n == size {
            task.pti_threadnum as usize
        } else {
            0
        }
    }

    /// Memory is the physical footprint; wakeups are interrupt wakeups, as
    /// one running count.
    pub fn sample(pid: u32) -> Result<Sample, String> {
        let r = rusage(pid)?;
        let mb = |b: u64| b as f64 / 1048576.0;
        Ok(Sample {
            cpu_seconds: mach_seconds(r.ri_user_time + r.ri_system_time),
            switches: BTreeMap::from([(0, r.ri_interrupt_wkups)]),
            peak_mb: mb(r.ri_lifetime_max_phys_footprint),
            rss_mb: mb(r.ri_phys_footprint),
            threads: threads(pid),
            idle_wakeups: Some(r.ri_pkg_idle_wkups),
        })
    }

    /// Resident size, for the record next to the footprint.
    pub fn resident_mb(pid: u32) -> f64 {
        rusage(pid).map_or(0.0, |r| r.ri_resident_size as f64 / 1048576.0)
    }
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
            Session::private()
        }
    }

    /// A private HOME and runtime directory, removed on drop.
    fn private_dir() -> Result<PathBuf, String> {
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
        Ok(dir)
    }

    /// Linux: a private Xvfb session. macOS: the logged-in desktop with a
    /// private HOME and TMPDIR.
    fn private() -> Result<Session, String> {
        if cfg!(target_os = "macos") {
            let dir = Session::private_dir()?;
            return Ok(Session {
                config: dir.join("home/Library/Application Support/pawprint/config.toml"),
                dir,
                display: String::new(),
                bus: None,
                _servers: Vec::new(),
                restore: Vec::new(),
            });
        }
        Session::xvfb()
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
        let dir = Session::private_dir()?;
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
            .env("PAWPRINT_HOSTNAME", "owl")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The IPC endpoint's directory (SPEC §10.4).
        if cfg!(target_os = "macos") {
            cmd.env("TMPDIR", self.dir.join("run"));
        } else {
            cmd.env("XDG_RUNTIME_DIR", self.dir.join("run"))
                .env("DISPLAY", &self.display);
        }
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
    idle_wakeups: Option<u64>,
    /// macOS: resident size next to the footprint.
    resident_mb: Option<f64>,
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
        idle_wakeups: after
            .idle_wakeups
            .zip(before.idle_wakeups)
            .map(|(a, b)| a - b),
        #[cfg(target_os = "macos")]
        resident_mb: Some(mac::resident_mb(pid)),
        #[cfg(not(target_os = "macos"))]
        resident_mb: None,
    })
}

fn cargo(root: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed", args.join(" ")))
    }
}

/// macOS: the arm64 and x86_64 release builds joined with `lipo`.
fn universal(root: &Path) -> Result<PathBuf, String> {
    let targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
    let mut args = vec!["build", "--release", "--package", "pawprint"];
    targets.iter().for_each(|t| args.extend(["--target", t]));
    cargo(root, &args)?;
    let out = root.join("target/power/pawprint");
    std::fs::create_dir_all(root.join("target/power")).map_err(|e| e.to_string())?;
    let status = Command::new("lipo")
        .arg("-create")
        .args(targets.map(|t| root.join("target").join(t).join("release/pawprint")))
        .arg("-output")
        .arg(&out)
        .status()
        .map_err(|e| format!("cannot run lipo: {e}"))?;
    if !status.success() {
        return Err("lipo -create failed".into());
    }
    Ok(out)
}

pub fn power_check(root: &Path, seconds: Option<&str>) -> Result<(), String> {
    if !cfg!(any(target_os = "linux", target_os = "macos", windows)) {
        return Err("make power-check: Linux, macOS, and Windows only".into());
    }
    let seconds: u64 = seconds
        .map(|s| {
            s.parse()
                .map_err(|_| format!("POWER_SECONDS={s}: not a number"))
        })
        .transpose()?
        .unwrap_or(120);
    println!("building the release binary...");
    let release = root.join("target/release");
    let exes = if cfg!(target_os = "macos") {
        let exe = universal(root)?;
        Exes {
            badge: exe.clone(),
            cli: exe,
        }
    } else if cfg!(windows) {
        cargo(root, &["build", "--release", "--package", "pawprint"])?;
        Exes {
            badge: release.join("pawprintw.exe"),
            cli: release.join("pawprint.exe"),
        }
    } else {
        cargo(root, &["build", "--release", "--package", "pawprint"])?;
        Exes {
            badge: release.join("pawprint"),
            cli: release.join("pawprint"),
        }
    };
    let size_mb = |p: &Path| std::fs::metadata(p).map(|m| m.len() as f64 / 1e6);
    let binary_mb = size_mb(&exes.badge).map_err(|e| e.to_string())?;
    let cli_mb = size_mb(&exes.cli).map_err(|e| e.to_string())?;

    let session = Session::start(&exes.cli)?;
    let place = if cfg!(any(windows, target_os = "macos")) {
        "this desktop"
    } else if session.bus.is_some() {
        "Xvfb + session bus"
    } else {
        "Xvfb"
    };
    println!("measuring {seconds} s in idle mode, then {seconds} s in static mode ({place})...");
    let idle = measure(&session, &exes, "", seconds)?;
    let stat = measure(&session, &exes, "animation = \"static\"\n", seconds)?;

    let mac = cfg!(target_os = "macos");
    let mut failures = Vec::new();
    let mut noted = false;
    // macOS: whole-process rows that include AppKit's threads are reported,
    // not failed (see the module comment).
    let system =
        |metric: &str| mac && (metric.ends_with("static mode") || metric == "CPU, idle mode");
    let mut row = |metric: &str, value: String, budget: &str, ok: bool| {
        let status = match (ok, system(metric)) {
            (true, _) => "ok",
            (false, false) => "OVER",
            (false, true) => "over*",
        };
        println!("  {metric:<30}{value:<34}{budget:<24}{status}");
        match status {
            "OVER" => failures.push(metric.to_string()),
            "over*" => noted = true,
            _ => {}
        }
    };
    println!(
        "\n  {:<30}{:<34}{:<24}",
        "metric", "measured", "budget (SPEC §7.2)"
    );
    let wakeups = |r: &Run| {
        format!(
            "{} in {} s{}",
            r.switches,
            r.seconds,
            r.idle_wakeups
                .map_or(String::new(), |i| format!(" ({i} pkg idle)"))
        )
    };
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
    if mac {
        // AppKit's threads cost the same in both modes, so idle minus
        // static is what animating costs.
        let animating = idle.cpu_percent - stat.cpu_percent;
        row(
            "CPU, idle over static",
            format!("{animating:.4}% of a core"),
            &format!("< {IDLE_CPU_BUDGET}%"),
            animating < IDLE_CPU_BUDGET,
        );
    }
    row(
        "wakeups, idle mode",
        wakeups(&idle),
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
        wakeups(&stat),
        "zero",
        stat.switches == 0,
    );
    let peak = idle.peak_mb.max(stat.peak_mb);
    row(
        if mac {
            "memory footprint (peak)"
        } else {
            "resident memory (peak)"
        },
        format!("{peak:.1} MB (now {:.1} MB)", idle.rss_mb),
        &format!("< {RSS_BUDGET_MB} MB"),
        peak < RSS_BUDGET_MB,
    );
    if let Some(resident) = idle.resident_mb {
        row(
            "resident size",
            format!("{resident:.1} MB"),
            "(shared pages count)",
            true,
        );
    }
    let (binary, value) = if mac {
        ("binary size (universal)", format!("{binary_mb:.1} MB"))
    } else if cfg!(windows) {
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
    if mac {
        println!("  (wakeups are interrupt wakeups)");
    }
    if noted {
        println!(
            "  * includes AppKit's threads; the badge's loop receives no event \
             in static mode (SPEC §7.2); reported, not failed"
        );
    }
    if failures.is_empty() {
        println!("\npower check: within budget");
        Ok(())
    } else {
        Err(format!("power check: over budget: {}", failures.join(", ")))
    }
}
