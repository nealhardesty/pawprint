//! Shared helpers for the display-server tests: an isolated environment for
//! the `pawprint` binary, child processes that die with the test, and screen
//! images compared against `pawprint render`.

#![allow(dead_code)] // each test file uses a different subset

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The test hostname: an owl labeled OWL.
pub const HOST: &str = "owl";

/// A child process stopped when dropped: asked to exit (SIGTERM, so Xvfb
/// removes its lock file and socket), then killed if it hasn't within a
/// second.
pub struct Proc(pub Child);

impl Drop for Proc {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.exited().is_none() {
            // SAFETY: kill only sends a signal to our own child.
            unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM) };
            let start = Instant::now();
            while self.exited().is_none() && start.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Proc {
    /// Exit status if it has exited.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.0.try_wait().ok().flatten()
    }
}

/// Fresh HOME, config, state, and runtime directories, removed on drop. The
/// runtime directory is short because Unix socket paths are limited to 108
/// bytes.
pub struct Env {
    pub dir: PathBuf,
    pub vars: Vec<(String, String)>,
}

impl Drop for Env {
    /// Kept after a failure, for its logs and screenshots.
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("test files kept in {}", self.dir.display());
            // CI only shows the output, so print the tail of every log.
            for entry in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "log") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let tail = &text[text.len().saturating_sub(3000)..];
                    eprintln!("--- {} ---\n{tail}", path.display());
                }
            }
        } else {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

impl Env {
    pub fn new(tag: &str) -> Env {
        let tmp = if cfg!(windows) {
            std::env::temp_dir()
        } else {
            PathBuf::from("/tmp")
        };
        let dir = tmp.join(format!("pp-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["home", "run"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let private = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(dir.join("run"), private).unwrap();
        }
        Env {
            dir,
            vars: Vec::new(),
        }
    }

    pub fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    pub fn runtime(&self) -> PathBuf {
        self.dir.join("run")
    }

    pub fn set(&mut self, k: &str, v: impl Into<String>) {
        self.vars.retain(|(key, _)| key != k);
        self.vars.push((k.into(), v.into()));
    }

    /// A command with only this environment's session variables.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        for k in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_TYPE",
            "SWAYSOCK",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "DBUS_SESSION_BUS_ADDRESS",
            "XAUTHORITY",
        ] {
            cmd.env_remove(k);
        }
        cmd.env("HOME", self.home())
            .env("XDG_RUNTIME_DIR", self.runtime())
            .env("PAWPRINT_HOSTNAME", HOST)
            .env("LANG", "C.UTF-8");
        for (k, v) in &self.vars {
            cmd.env(k, v);
        }
        cmd
    }

    pub fn spawn(&self, program: &str, args: &[&str]) -> Proc {
        let mut cmd = self.command(program);
        cmd.args(args);
        self.spawn_command(cmd)
    }

    /// Start `cmd` with its output in `<program>.log`.
    pub fn spawn_command(&self, mut cmd: Command) -> Proc {
        let program = Path::new(cmd.get_program())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let log = std::fs::File::create(self.dir.join(format!("{program}.log"))).unwrap();
        Proc(
            cmd.stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap_or_else(|e| panic!("cannot start {program}: {e} (is it installed?)")),
        )
    }

    pub fn pawprint(&self, args: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_pawprint"))
            .args(args)
            .output()
            .unwrap()
    }

    /// `pawprint run` in the foreground, as autostart runs it.
    pub fn run_badge(&self) -> Proc {
        self.spawn(env!("CARGO_BIN_EXE_pawprint"), &["run", "--verbose"])
    }

    /// Run a CLI command that must succeed.
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.pawprint(args);
        assert!(
            out.status.success(),
            "pawprint {args:?} failed: {}{}\nbadge log:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            self.log()
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("pawprint.log")).unwrap_or_default()
    }

    /// The badge rectangle from `pawprint status`, once it is placed.
    pub fn badge_rect(&self) -> Option<[i32; 4]> {
        let out = self.pawprint(&["status"]);
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().find(|l| l.starts_with("badge"))?;
        // "badge       64x64 at 1200,16 (scale 1)"
        let mut words = line.split_whitespace().skip(1);
        let (w, h) = words.next()?.split_once('x')?;
        let (x, y) = words.nth(1)?.split_once(',')?;
        Some([
            x.parse().ok()?,
            y.parse().ok()?,
            w.parse().ok()?,
            h.parse().ok()?,
        ])
    }

    pub fn wait_rect(&self, want: [i32; 4]) {
        let start = Instant::now();
        while self.badge_rect() != Some(want) {
            if start.elapsed() > Duration::from_secs(10) {
                panic!(
                    "badge at {:?}, want {want:?}\nbadge log:\n{}",
                    self.badge_rect(),
                    self.log()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn state(&self) -> String {
        let dir = if cfg!(target_os = "macos") {
            self.home().join("Library/Application Support/pawprint")
        } else {
            self.home().join(".local/state/pawprint")
        };
        std::fs::read_to_string(dir.join("state.toml")).unwrap_or_default()
    }

    pub fn config_file(&self) -> PathBuf {
        let dir = if cfg!(target_os = "macos") {
            self.home().join("Library/Application Support/pawprint")
        } else {
            self.home().join(".config/pawprint")
        };
        dir.join("config.toml")
    }

    pub fn config(&self) -> String {
        std::fs::read_to_string(self.config_file()).unwrap_or_default()
    }

    pub fn write_config(&self, text: &str) {
        let path = self.config_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// What the badge should look like at scale 1: `pawprint render`.
    pub fn expected(&self, size: u32) -> Image {
        let path = self.dir.join(format!("expected-{size}.png"));
        self.ok(&[
            "render",
            HOST,
            "--size",
            &size.to_string(),
            "--out",
            path.to_str().unwrap(),
        ]);
        Image::load(&path)
    }
}

/// Poll `f` until it returns `Some`, for up to 10 seconds.
pub fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// An RGBA image.
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn load(path: &Path) -> Image {
        let mut decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        decoder.set_transformations(
            png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
        );
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba, "{}", path.display());
        buf.truncate(info.buffer_size());
        Image {
            width: info.width,
            height: info.height,
            rgba: buf,
        }
    }

    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba[i..i + 4].try_into().unwrap()
    }

    /// Bounding box of pixels that differ from `background`.
    pub fn bbox(&self, background: [u8; 3]) -> Option<[i32; 4]> {
        self.bbox_in([0, 0, self.width as i32, self.height as i32], background)
    }

    /// The same, looking only inside `area`.
    pub fn bbox_in(&self, area: [i32; 4], background: [u8; 3]) -> Option<[i32; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in area[1] as u32..(area[1] + area[3]) as u32 {
            for x in area[0] as u32..(area[0] + area[2]) as u32 {
                if self.px(x, y)[..3] != background {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        (x0 != u32::MAX).then(|| {
            [
                x0 as i32,
                y0 as i32,
                (x1 - x0 + 1) as i32,
                (y1 - y0 + 1) as i32,
            ]
        })
    }

    /// Bounding box of the opaque pixels.
    pub fn opaque_bbox(&self) -> [i32; 4] {
        let mut copy = self.clone();
        for p in copy.rgba.chunks_mut(4) {
            if p[3] == 0 {
                p.copy_from_slice(&[1, 2, 3, 0]);
            }
        }
        copy.bbox([1, 2, 3]).expect("an opaque pixel")
    }

    pub fn opaque_colors(&self) -> std::collections::BTreeSet<[u8; 3]> {
        self.rgba
            .chunks(4)
            .filter(|p| p[3] == 255)
            .map(|p| [p[0], p[1], p[2]])
            .collect()
    }

    pub fn opaque_count(&self) -> usize {
        self.rgba.chunks(4).filter(|p| p[3] == 255).count()
    }

    fn in_set(&self, x: u32, y: u32, set: &std::collections::BTreeSet<[u8; 3]>) -> bool {
        let p = self.px(x, y);
        set.contains(&[p[0], p[1], p[2]])
    }

    /// Bounding box, inside `area`, of pixels whose color is in `set`.
    pub fn bbox_of(
        &self,
        area: [i32; 4],
        set: &std::collections::BTreeSet<[u8; 3]>,
    ) -> Option<[i32; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, 0, 0);
        for y in area[1]..area[1] + area[3] {
            for x in area[0]..area[0] + area[2] {
                if self.in_set(x as u32, y as u32, set) {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        (x0 != i32::MAX).then(|| [x0, y0, x1 - x0 + 1, y1 - y0 + 1])
    }

    /// How many pixels inside `area` have a color in `set`.
    pub fn count_of(&self, area: [i32; 4], set: &std::collections::BTreeSet<[u8; 3]>) -> usize {
        (area[1]..area[1] + area[3])
            .flat_map(|y| (area[0]..area[0] + area[2]).map(move |x| (x, y)))
            .filter(|&(x, y)| self.in_set(x as u32, y as u32, set))
            .count()
    }

    /// Distinct RGB colors inside a rectangle.
    pub fn colors(&self, r: [i32; 4]) -> std::collections::BTreeSet<[u8; 3]> {
        let mut set = std::collections::BTreeSet::new();
        for y in r[1]..r[1] + r[3] {
            for x in r[0]..r[0] + r[2] {
                let p = self.px(x as u32, y as u32);
                set.insert([p[0], p[1], p[2]]);
            }
        }
        set
    }
}

/// The screen shows `badge` at (x, y): every opaque badge pixel exactly, and
/// the background through every transparent one.
pub fn assert_badge_at(screen: &Image, badge: &Image, x: i32, y: i32, background: [u8; 3]) {
    let mut wrong = 0;
    let mut first = None;
    for by in 0..badge.height {
        for bx in 0..badge.width {
            let b = badge.px(bx, by);
            let want = if b[3] == 255 {
                [b[0], b[1], b[2]]
            } else {
                background
            };
            let got = screen.px((x + bx as i32) as u32, (y + by as i32) as u32);
            if got[..3] != want {
                wrong += 1;
                first.get_or_insert((bx, by, want, got));
            }
        }
    }
    assert_eq!(
        wrong, 0,
        "{wrong} badge pixels differ at ({x}, {y}); first (x, y, want, got): {first:?}"
    );
}
