//! Shared Linux pieces (SPEC §10.6): backend selection (§3.2), the `calloop`
//! event loop both backends run on, the StatusNotifierItem tray, scale
//! helpers, and the Linux half of `pawprint doctor`.

use calloop::EventLoop;
use calloop::channel::{self, Event as ChannelEvent};
use pawprint_core::badge::Command;
use pawprint_core::config::Backend;
use pawprint_core::movemode::KeyCode;
use pawprint_core::surface::{DisplayInfo, Surface};
use pawprint_core::ui::{Input, UiKey};

use super::{sni, wayland, x11};
use crate::doctor::Report;
use crate::shell::Shell;
use crate::{CmdResult, exit, ipc};

/// Which backend to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Wayland,
    /// `gnome`: the GNOME profile, through XWayland (SPEC §11.5.1).
    X11 {
        gnome: bool,
    },
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Choice::Wayland => "wayland",
            Choice::X11 { gnome: false } => "x11",
            Choice::X11 { gnome: true } => "x11 (gnome profile)",
        })
    }
}

/// What the session looks like.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub wayland_display: bool,
    pub x_display: bool,
    pub desktop: String,
    /// The Wayland compositor advertises `zwlr_layer_shell_v1`.
    pub layer_shell: bool,
}

impl Env {
    pub fn detect() -> Env {
        let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
        let wayland_display = set("WAYLAND_DISPLAY");
        Env {
            wayland_display,
            x_display: set("DISPLAY"),
            desktop: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            layer_shell: wayland_display
                && wayland::globals().is_ok_and(|g| g.iter().any(|i| i == "zwlr_layer_shell_v1")),
        }
    }

    fn desktop_has(&self, name: &str) -> bool {
        self.desktop
            .split(':')
            .any(|d| d.eq_ignore_ascii_case(name))
    }

    pub fn gnome(&self) -> bool {
        self.wayland_display && self.desktop_has("GNOME")
    }
}

/// The backend, why it was chosen, and an optional warning; `choice` is
/// `None` when nothing can run (exit code 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub choice: Option<Choice>,
    pub reason: String,
    pub warning: Option<String>,
}

/// SPEC §3.2. The Plan B presenter (§11.5.3) would slot in before the GNOME
/// step; it is built only if the GNOME checks fail.
pub fn select(configured: Backend, env: &Env) -> Selection {
    let pick = |choice, reason: &str| Selection {
        choice: Some(choice),
        reason: reason.into(),
        warning: None,
    };
    match configured {
        Backend::Wayland => {
            return pick(Choice::Wayland, "backend = \"wayland\" in config");
        }
        Backend::X11 => {
            return pick(
                Choice::X11 { gnome: env.gnome() },
                "backend = \"x11\" in config",
            );
        }
        Backend::Auto | Backend::Windows | Backend::Macos => {}
    }
    if env.wayland_display && env.layer_shell {
        return pick(
            Choice::Wayland,
            "WAYLAND_DISPLAY is set and the compositor has zwlr_layer_shell_v1",
        );
    }
    if env.gnome() && env.x_display {
        return pick(
            Choice::X11 { gnome: true },
            "GNOME on Wayland has no layer-shell; using XWayland (DISPLAY is set)",
        );
    }
    if env.x_display {
        let mut s = pick(Choice::X11 { gnome: false }, "DISPLAY is set");
        if env.wayland_display {
            s.reason = "DISPLAY is set; the Wayland compositor has no zwlr_layer_shell_v1".into();
            if !env.desktop_has("KDE") {
                s.warning = Some(format!(
                    "running through XWayland on an untested compositor ({}); \
                     always-on-top and click-through depend on it",
                    if env.desktop.is_empty() {
                        "XDG_CURRENT_DESKTOP unset"
                    } else {
                        &env.desktop
                    }
                ));
            }
        }
        return s;
    }
    Selection {
        choice: None,
        reason: if env.wayland_display {
            "the Wayland compositor has no zwlr_layer_shell_v1 and DISPLAY is not set (no XWayland)"
        } else {
            "neither WAYLAND_DISPLAY nor DISPLAY is set"
        }
        .into(),
        warning: None,
    }
}

pub fn run(shell: Shell, listener: ipc::Listener) -> CmdResult {
    start(shell, Some(listener))
}

/// `pawprint preview` and `pawprint settings`: the same backends with a
/// normal window, without IPC.
pub fn window(shell: Shell) -> CmdResult {
    start(shell, None)
}

fn start(mut shell: Shell, listener: Option<ipc::Listener>) -> CmdResult {
    let configured = shell.configured_backend();
    if matches!(configured, Backend::Windows | Backend::Macos) {
        log::warn!("backend = \"{configured}\" does not exist on Linux; using auto");
    }
    let selection = select(configured, &Env::detect());
    if let Some(w) = &selection.warning {
        log::warn!("{w}");
    }
    let Some(choice) = selection.choice else {
        return Err((
            exit::NO_SESSION,
            format!(
                "no usable graphical session: {}. Run `pawprint doctor`.",
                selection.reason
            ),
        ));
    };
    shell.set_backend(choice.to_string());
    log::info!("backend reason: {}", selection.reason);
    match choice {
        Choice::Wayland => wayland::run(shell, listener),
        Choice::X11 { gnome } => x11::run(shell, listener, gnome),
    }
}

/// What wakes the loop besides the display connection: IPC requests and
/// tray menu choices, both posted from their own threads.
pub enum Inbox {
    Ipc(ipc::Incoming),
    Menu(Command),
}

/// What both backends hand to [`drive`]: the shell and the surface, split so
/// the shell can drive the surface.
pub trait App: 'static {
    type Surface: Surface;
    fn parts(&mut self) -> (&mut Shell, &mut Self::Surface);
    /// Send buffered requests to the display server and handle any events it
    /// already delivered. An error means the connection is gone.
    fn flush(&mut self) -> Result<(), String>;
}

/// The loop every Linux backend runs (SPEC §10.1): IPC requests and tray
/// choices arrive on a calloop channel, the display connection is a calloop
/// source the backend inserted, and the loop sleeps until the next native
/// event or the core's deadline. `listener` is `None` for a preview, which
/// also gets no tray.
pub fn drive<A: App>(
    mut event_loop: EventLoop<'static, A>,
    app: &mut A,
    listener: Option<ipc::Listener>,
) -> CmdResult {
    let fail = |e: String| (exit::ERROR, e);
    let (tx, rx) = channel::channel::<Inbox>();
    event_loop
        .handle()
        .insert_source(rx, |event, _, app: &mut A| {
            let (shell, surface) = app.parts();
            match event {
                ChannelEvent::Msg(Inbox::Ipc(incoming)) => shell.request(surface, incoming),
                ChannelEvent::Msg(Inbox::Menu(command)) => shell.menu(surface, command),
                ChannelEvent::Closed => {}
            }
        })
        .map_err(|e| fail(format!("event loop: {e}")))?;
    // The tray joins once the badge is on screen, so the session bus never
    // delays it.
    let mut tray = listener.is_some().then(|| sni::Sni::new(tx.clone()));
    if let Some(listener) = listener {
        ipc::serve(listener, move |incoming| {
            tx.send(Inbox::Ipc(incoming)).is_ok()
        });
    }
    loop {
        app.flush().map_err(fail)?;
        if let Some(t) = tray.take() {
            app.parts().0.set_tray(Box::new(t));
        }
        let (shell, _) = app.parts();
        if shell.done() {
            return Ok(());
        }
        let timeout = shell.timeout();
        event_loop
            .dispatch(timeout, app)
            .map_err(|e| fail(format!("display connection: {e}")))?;
        let (shell, surface) = app.parts();
        shell.tick(surface);
    }
}

/// Move mode's keys (SPEC §6.3) for both backends: the X keysym (X11 reads
/// keysyms), its name in an XKB keymap (Wayland reads the keymap text), and
/// the core's key.
pub const MOVE_KEYS: [(u32, &str, KeyCode); 7] = [
    (0xff51, "Left", KeyCode::Left),
    (0xff52, "Up", KeyCode::Up),
    (0xff53, "Right", KeyCode::Right),
    (0xff54, "Down", KeyCode::Down),
    (0xff0d, "Return", KeyCode::Enter),
    (0xff8d, "KP_Enter", KeyCode::Enter),
    (0xff1b, "Escape", KeyCode::Escape),
];

/// The normal window's input for a key (SPEC §8.7), for both backends.
/// `syms` is the key's keysyms without and with Shift; Caps Lock shifts
/// letters only, and Ctrl+letter is a shortcut.
pub fn window_input(syms: (u32, u32), shift: bool, caps: bool, ctrl: bool) -> Option<Input> {
    let (lower, upper) = syms;
    let key = match lower {
        0xff51 => Some(UiKey::Left),
        0xff52 => Some(UiKey::Up),
        0xff53 => Some(UiKey::Right),
        0xff54 => Some(UiKey::Down),
        0xff50 => Some(UiKey::Home),
        0xff57 => Some(UiKey::End),
        0xff55 => Some(UiKey::PageUp),
        0xff56 => Some(UiKey::PageDown),
        0xff08 => Some(UiKey::Backspace),
        0xffff => Some(UiKey::Delete),
        0xff0d | 0xff8d => Some(UiKey::Enter),
        0xff1b => Some(UiKey::Escape),
        0xff09 | 0xfe20 => Some(UiKey::Tab),
        _ => None,
    };
    if let Some(k) = key {
        return Some(Input::Key(k));
    }
    let letter = keysym_char(lower).filter(char::is_ascii_alphabetic);
    if ctrl {
        return letter.map(|c| Input::Shortcut(c.to_ascii_lowercase()));
    }
    let upper = if upper == 0 { lower } else { upper };
    let sym = if shift != (caps && letter.is_some()) {
        upper
    } else {
        lower
    };
    keysym_char(sym)
        .filter(|c| !c.is_control())
        .map(Input::Text)
}

/// The character a keysym types: Latin-1 keysyms are their code point,
/// and `0x1000000 + U` is Unicode `U`.
fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        0x100_0000.. => char::from_u32(sym - 0x100_0000),
        _ => None,
    }
}

/// A keysym from its name in an XKB keymap (`a`, `A`, `comma`, `U00E9`,
/// `Left`), for the Wayland keymap reader; `None` for names text never
/// needs.
pub fn keysym_from_name(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c.is_ascii_graphic().then_some(c as u32);
    }
    if let Some(hex) = name.strip_prefix('U').filter(|h| h.len() >= 4) {
        return u32::from_str_radix(hex, 16).ok().map(|u| u + 0x100_0000);
    }
    if let Some(hex) = name.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok();
    }
    // Move mode's keys (arrows, Return, Escape) are in MOVE_KEYS.
    const NAMES: &[(&str, u32)] = &[
        ("space", 0x20),
        ("exclam", 0x21),
        ("quotedbl", 0x22),
        ("numbersign", 0x23),
        ("dollar", 0x24),
        ("percent", 0x25),
        ("ampersand", 0x26),
        ("apostrophe", 0x27),
        ("parenleft", 0x28),
        ("parenright", 0x29),
        ("asterisk", 0x2a),
        ("plus", 0x2b),
        ("comma", 0x2c),
        ("minus", 0x2d),
        ("period", 0x2e),
        ("slash", 0x2f),
        ("colon", 0x3a),
        ("semicolon", 0x3b),
        ("less", 0x3c),
        ("equal", 0x3d),
        ("greater", 0x3e),
        ("question", 0x3f),
        ("at", 0x40),
        ("bracketleft", 0x5b),
        ("backslash", 0x5c),
        ("bracketright", 0x5d),
        ("asciicircum", 0x5e),
        ("underscore", 0x5f),
        ("grave", 0x60),
        ("braceleft", 0x7b),
        ("bar", 0x7c),
        ("braceright", 0x7d),
        ("asciitilde", 0x7e),
        ("Home", 0xff50),
        ("Prior", 0xff55),
        ("Page_Up", 0xff55),
        ("Next", 0xff56),
        ("Page_Down", 0xff56),
        ("End", 0xff57),
        ("BackSpace", 0xff08),
        ("Tab", 0xff09),
        ("ISO_Left_Tab", 0xfe20),
        ("Delete", 0xffff),
    ];
    NAMES
        .iter()
        .copied()
        .chain(MOVE_KEYS.iter().map(|&(sym, name, _)| (name, sym)))
        .find(|&(n, _)| n == name)
        .map(|(_, sym)| sym)
}

/// `Xft.dpi` from the X resource database string, if set.
pub fn xft_dpi(resources: &str) -> Option<f64> {
    resources.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "Xft.dpi").then(|| v.trim().parse().ok())?
    })
}

/// X11 scale: `Xft.dpi` ÷ 96, rounded to 0.25 (SPEC §11.3.3).
pub fn dpi_scale(dpi: Option<f64>) -> f64 {
    dpi.filter(|d| *d > 0.0)
        .map_or(1.0, |d| ((d / 96.0) * 4.0).round().max(1.0) / 4.0)
}

/// Wayland scale from an output's physical mode and logical size, rounded
/// to the 1/120 steps of the fractional-scale protocol.
pub fn mode_scale(physical_width: i32, logical_width: i32) -> f64 {
    if logical_width <= 0 || physical_width <= 0 {
        return 1.0;
    }
    (f64::from(physical_width) / f64::from(logical_width) * 120.0).round() / 120.0
}

pub fn doctor(report: &mut Report) {
    let env = Env::detect();
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| "(unset)".into());
    report.section("Session");
    report.row("XDG_SESSION_TYPE", var("XDG_SESSION_TYPE"));
    report.row("XDG_CURRENT_DESKTOP", var("XDG_CURRENT_DESKTOP"));
    report.row("WAYLAND_DISPLAY", var("WAYLAND_DISPLAY"));
    report.row("DISPLAY", var("DISPLAY"));
    if env.wayland_display {
        match wayland::globals() {
            Ok(g) => {
                let has = |i: &str| {
                    if g.iter().any(|x| x == i) {
                        "yes"
                    } else {
                        "no"
                    }
                };
                report.row("layer-shell", has("zwlr_layer_shell_v1"));
                report.row("fractional scale", has("wp_fractional_scale_manager_v1"));
                report.row("viewporter", has("wp_viewporter"));
                report.row("kde output order", has("kde_output_order_v1"));
            }
            Err(e) => report.problem(format!("cannot connect to Wayland: {e}")),
        }
    }

    let selection = select(crate::doctor::settings().backend.value, &env);
    report.section("Backend");
    match selection.choice {
        Some(c) => report.row("backend", c.to_string()),
        None => report.problem(format!(
            "no usable backend: {}; PAWPRINT needs a graphical session",
            selection.reason
        )),
    }
    report.row("why", &selection.reason);
    if let Some(w) = selection.warning {
        report.problem(w);
    }
    let displays = match selection.choice {
        Some(Choice::Wayland) => wayland::doctor(report),
        Some(Choice::X11 { .. }) => x11::doctor(report),
        None => Vec::new(),
    };
    report.displays(&displays);
    if env.gnome() {
        gnome_scaling(report);
    }
    tray(report, &env);
}

/// SPEC §11.5.1: with fractional scaling, Mutter upscales XWayland windows
/// (blurry) unless `xwayland-native-scaling` is on (GNOME 47+, F28).
fn gnome_scaling(report: &mut Report) {
    let outputs = wayland::outputs().unwrap_or_default();
    let fractional: Vec<&DisplayInfo> = outputs.iter().filter(|d| d.scale.fract() != 0.0).collect();
    let features = command_output(
        "gsettings",
        &["get", "org.gnome.mutter", "experimental-features"],
    );
    report.row(
        "mutter features",
        features.as_deref().unwrap_or("(gsettings unavailable)"),
    );
    if fractional.is_empty() {
        report.row("gnome scaling", "integer scale on every monitor: crisp");
        return;
    }
    let native = features
        .as_deref()
        .is_some_and(|f| f.contains("xwayland-native-scaling"));
    let names: Vec<String> = fractional
        .iter()
        .map(|d| format!("{} at {}", d.name, d.scale))
        .collect();
    if native {
        report.row(
            "gnome scaling",
            format!(
                "fractional ({}), xwayland-native-scaling on: crisp",
                names.join(", ")
            ),
        );
        return;
    }
    let mut list: Vec<String> = features
        .as_deref()
        .map(|f| {
            f.trim_start_matches("@as")
                .trim()
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    list.push("'xwayland-native-scaling'".into());
    let version = command_output("gnome-shell", &["--version"]).unwrap_or_default();
    report.problem(format!(
        "fractional scaling ({}) blurs the badge: Mutter upscales XWayland windows. \
         On GNOME 47 or newer, fix it with\n      gsettings set org.gnome.mutter experimental-features \"[{}]\"\n    \
         then log out and back in. {}",
        names.join(", "),
        list.join(", "),
        if version.contains(" 46") {
            "This is GNOME 46, which lacks that feature: use 100% or 200% scaling for a crisp badge."
        } else {
            ""
        }
    ));
}

/// The StatusNotifierItem watcher on the session bus (SPEC §11.4, F29).
/// Without one the badge runs without a tray and says nothing; this is
/// where the user finds out.
fn tray(report: &mut Report, env: &Env) {
    if !crate::doctor::settings().tray.value {
        report.row("tray", "off (tray = false in config)");
        return;
    }
    match sni::watcher_present() {
        Ok(true) => report.row("tray", "StatusNotifierItem host available"),
        Ok(false) if env.gnome() => report.problem(
            "no tray: this GNOME has no AppIndicator extension (Ubuntu ships one; elsewhere \
             install \"AppIndicator and KStatusNotifierItem Support\"). The CLI does everything \
             the tray does (F29).",
        ),
        Ok(false) => report.problem(
            "no tray: nothing on the session bus shows StatusNotifierItems \
             (org.kde.StatusNotifierWatcher). The CLI does everything the tray does.",
        ),
        Err(e) => report.problem(format!("no tray: cannot reach the session bus ({e})")),
    }
}

fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(wayland: bool, x: bool, desktop: &str, layer_shell: bool) -> Env {
        Env {
            wayland_display: wayland,
            x_display: x,
            desktop: desktop.into(),
            layer_shell,
        }
    }

    #[test]
    fn selection_follows_spec_3_2() {
        let auto = |e: Env| select(Backend::Auto, &e);
        assert_eq!(
            auto(env(true, true, "KDE", true)).choice,
            Some(Choice::Wayland)
        );
        assert_eq!(
            auto(env(true, false, "sway", true)).choice,
            Some(Choice::Wayland)
        );
        assert_eq!(
            auto(env(true, true, "ubuntu:GNOME", false)).choice,
            Some(Choice::X11 { gnome: true })
        );
        let plain = auto(env(false, true, "", false));
        assert_eq!(plain.choice, Some(Choice::X11 { gnome: false }));
        assert_eq!(plain.warning, None, "native X11 and xrdp sessions are fine");
        let odd = auto(env(true, true, "weston", false));
        assert_eq!(odd.choice, Some(Choice::X11 { gnome: false }));
        assert!(
            odd.warning.is_some(),
            "XWayland on an untested compositor warns"
        );
        assert_eq!(auto(env(true, false, "GNOME", false)).choice, None);
        assert_eq!(auto(env(false, false, "", false)).choice, None);
    }

    #[test]
    fn configured_backend_wins() {
        let e = env(true, true, "GNOME", false);
        assert_eq!(select(Backend::Wayland, &e).choice, Some(Choice::Wayland));
        assert_eq!(
            select(Backend::X11, &e).choice,
            Some(Choice::X11 { gnome: true })
        );
        assert_eq!(
            select(Backend::Windows, &env(false, true, "", false)).choice,
            Some(Choice::X11 { gnome: false }),
            "another OS's backend falls back to auto"
        );
    }

    #[test]
    fn keys_type_text_and_edit() {
        let a = (0x61, 0x41);
        assert_eq!(window_input(a, false, false, false), Some(Input::Text('a')));
        assert_eq!(window_input(a, true, false, false), Some(Input::Text('A')));
        assert_eq!(window_input(a, false, true, false), Some(Input::Text('A')));
        assert_eq!(window_input(a, true, true, false), Some(Input::Text('a')));
        assert_eq!(
            window_input(a, false, false, true),
            Some(Input::Shortcut('a'))
        );
        let one = (0x31, 0x21);
        assert_eq!(
            window_input(one, false, true, false),
            Some(Input::Text('1')),
            "caps lock leaves digits"
        );
        assert_eq!(
            window_input(one, true, false, false),
            Some(Input::Text('!'))
        );
        assert_eq!(
            window_input((0xff08, 0), false, false, false),
            Some(Input::Key(UiKey::Backspace))
        );
        assert_eq!(
            window_input((0xff0d, 0), true, false, true),
            Some(Input::Key(UiKey::Enter))
        );
        assert_eq!(
            window_input((0x20, 0), false, false, false),
            Some(Input::Text(' '))
        );
        assert_eq!(
            window_input((0x0100_00e9, 0x0100_00c9), true, false, false),
            Some(Input::Text('É'))
        );
        assert_eq!(
            window_input((0xffe1, 0), false, false, false),
            None,
            "Shift itself"
        );
        for (name, sym) in [
            ("a", 0x61),
            ("Z", 0x5a),
            ("comma", 0x2c),
            ("U00E9", 0x0100_00e9),
            ("Escape", 0xff1b),
            ("KP_Enter", 0xff8d),
            ("BackSpace", 0xff08),
        ] {
            assert_eq!(keysym_from_name(name), Some(sym), "{name}");
        }
        assert_eq!(keysym_from_name("XF86AudioMute"), None);
    }

    #[test]
    fn scale_helpers() {
        let res = "Xft.antialias:\t1\nXft.dpi:\t144\nXft.hinting:\t1\n";
        assert_eq!(xft_dpi(res), Some(144.0));
        assert_eq!(xft_dpi("Xcursor.size: 24"), None);
        assert_eq!(dpi_scale(Some(144.0)), 1.5);
        assert_eq!(dpi_scale(Some(120.0)), 1.25);
        assert_eq!(dpi_scale(Some(100.0)), 1.0);
        assert_eq!(dpi_scale(None), 1.0);
        assert_eq!(mode_scale(2560, 2048), 1.25);
        assert_eq!(mode_scale(3840, 1920), 2.0);
        assert_eq!(mode_scale(1920, 0), 1.0);
    }
}
