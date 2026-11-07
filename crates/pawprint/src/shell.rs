//! The imperative shell around `pawprint_core::badge::Badge` (SPEC §10.1),
//! written once for every backend.
//!
//! A backend owns its native loop and a [`Surface`]. It calls
//! [`Shell::start`] once, feeds translated native events to
//! [`Shell::handle`], hands IPC requests to [`Shell::request`] and tray
//! choices to [`Shell::menu`], sleeps for at most [`Shell::timeout`], calls
//! [`Shell::tick`] after every wakeup, and stops when [`Shell::done`] is
//! true. Everything else (state saving, config reloads and edits, the tray
//! state, status, logging) happens here.

use std::time::{Duration, Instant, SystemTime};

use pawprint_core::badge::{Badge, Command, Effect, Event, Inputs};
use pawprint_core::catalog::bundle;
use pawprint_core::config::{Animation, Layer};
use pawprint_core::config_edit;
use pawprint_core::geom::PhysicalRect;
use pawprint_core::layout::Anchor;
use pawprint_core::menu::TrayState;
use pawprint_core::render::Frame;
use pawprint_core::state::State;
use pawprint_core::surface::{self, DisplayInfo, Surface};

use crate::ipc::{Incoming, Request, Response, Status};
use crate::paths::Paths;
use crate::{clock, commands, exit, hostname};

/// A platform's tray icon (Windows), menu-bar item (macOS), or
/// StatusNotifierItem (Linux). It draws what it is given; menu choices come
/// back to the loop and into [`Shell::menu`].
pub trait Tray {
    /// Show, update, or (when `state.visible` is false) remove the icon.
    fn show(&mut self, state: &TrayState);
}

/// `pawprint preview` (SPEC §9): a normal window around the badge, on this
/// backdrop so its transparent pixels show, with this margin.
pub const PREVIEW_BACKDROP: [u8; 3] = [0x5a, 0x60, 0x6b];
pub const PREVIEW_MARGIN: u32 = 16;

pub struct Shell {
    badge: Badge<'static>,
    paths: Paths,
    session: String,
    backend: String,
    /// The display set last given to the core, to drop repeats.
    displays: Vec<DisplayInfo>,
    started: bool,
    done: bool,
    tray: Option<Box<dyn Tray>>,
    tray_state: Option<TrayState>,
    /// The config file's modification time when it was last read (SPEC §8.5).
    config_mtime: Option<SystemTime>,
    preview: bool,
}

impl Shell {
    pub fn new(paths: Paths, session: String, flags: Layer) -> Result<Shell, (u8, String)> {
        let (state, warning) = State::load(&paths.state_file);
        warning.into_iter().for_each(|w| log::warn!("{w}"));
        Shell::build(paths, session, flags, state, false)
    }

    /// `pawprint preview <id>`: that mascot at `size`, centered, playing
    /// every action in turn. It reads the config (for the label, accent, and
    /// backend) but never writes the state file, serves no IPC, and shows no
    /// tray.
    pub fn preview(paths: Paths, mascot: &str, size: u32) -> Result<Shell, (u8, String)> {
        let flags = Layer {
            mascot: Some(mascot.to_string()),
            size: Some(size),
            position: Some(Anchor::Center),
            offset: Some([0, 0]),
            animation: Some(Animation::Idle),
            interval: Some([1, 1]),
            tray: Some(false),
            ..Layer::default()
        };
        Shell::build(paths, "preview".into(), flags, State::default(), true)
    }

    fn build(
        paths: Paths,
        session: String,
        flags: Layer,
        state: State,
        preview: bool,
    ) -> Result<Shell, (u8, String)> {
        let hostname = hostname::detect().map_err(|e| (exit::ERROR, e))?;
        let config_mtime = modified(&paths);
        let (config, config_note) = commands::load_config(&paths);
        log::info!("hostname {hostname:?}, config {config_note}");
        let inputs = Inputs {
            hostname,
            config,
            state,
            flags,
            seed: clock::seed(),
            preview,
        };
        let catalog = commands::catalog(&paths);
        catalog.warnings().iter().for_each(|w| log::warn!("{w}"));
        let badge = Badge::new(catalog, bundle(), inputs, Instant::now());
        let r = badge.resolved();
        log::info!(
            "mascot {} ({}), label {:?}",
            r.mascot,
            r.trace.last().map_or("", |s| s.outcome.as_str()),
            r.label
        );
        r.warnings.iter().for_each(|w| log::warn!("config: {w}"));
        Ok(Shell {
            badge,
            paths,
            session,
            backend: String::new(),
            displays: Vec::new(),
            started: false,
            done: false,
            tray: None,
            tray_state: None,
            config_mtime,
            preview,
        })
    }

    /// True for `pawprint preview`: backends open a normal, focusable,
    /// closable window instead of the overlay.
    pub fn is_preview(&self) -> bool {
        self.preview
    }

    /// Attach the platform's tray. It is shown at once if the core already
    /// described it.
    pub fn set_tray(&mut self, mut tray: Box<dyn Tray>) {
        if let Some(state) = &self.tray_state {
            tray.show(state);
        }
        self.tray = Some(tray);
    }

    /// The configured backend setting, for backend selection.
    pub fn configured_backend(&self) -> pawprint_core::config::Backend {
        self.badge.resolved().settings.backend.value
    }

    /// Name the backend for `status` and the log, e.g. `x11 (gnome profile)`.
    pub fn set_backend(&mut self, name: impl Into<String>) {
        self.backend = name.into();
        log::info!("backend {}", self.backend);
    }

    /// Show the badge. Call once, when the surface can report its displays.
    pub fn start(&mut self, surface: &mut impl Surface) {
        self.displays = surface.displays();
        log_displays(&self.displays);
        let effects = self.badge.start(self.displays.clone(), clock::now());
        self.started = true;
        self.run(surface, effects);
    }

    /// A tray menu choice. Size and Animation are saved the way `pawprint
    /// set` saves them, into this host's `[hosts."<short>"]` table, and take
    /// effect through a reload, so they survive restarts (SPEC §8.6). If the
    /// config can't be written they still apply until the badge restarts.
    /// Everything else is the same command IPC sends.
    pub fn menu(&mut self, surface: &mut impl Surface, command: Command) {
        log::info!("tray: {command:?}");
        let edit = match command {
            Command::SetSize(n) => Some(("size", n.to_string())),
            Command::SetAnimation(a) => Some(("animation", a.to_string())),
            _ => None,
        };
        if let Some((key, value)) = edit.filter(|_| !self.preview) {
            let short = self.badge.resolved().short.clone();
            match commands::edit_config(&self.paths, |text| {
                config_edit::set(text, &short, key, &value)
            }) {
                Ok(()) => {
                    log::info!("saved {key} = {value} in [hosts.{short:?}]");
                    return self.handle(surface, Event::Command(Command::Reload));
                }
                Err(e) => {
                    log::warn!("cannot save {key} ({e}); it applies until the badge restarts")
                }
            }
        }
        self.handle(surface, Event::Command(command));
    }

    /// Feed one translated native event to the core. Events before `start`
    /// and repeated identical display sets are dropped, so backends can
    /// report every native change without filtering.
    pub fn handle(&mut self, surface: &mut impl Surface, event: Event) {
        if !self.started || self.done {
            return;
        }
        if let Event::DisplaysChanged(displays) = &event {
            if *displays == self.displays {
                return;
            }
            self.displays.clone_from(displays);
            log_displays(displays);
        }
        let effects = self.badge.handle(event, clock::now());
        self.run(surface, effects);
    }

    /// Answer one IPC request.
    pub fn request(&mut self, surface: &mut impl Surface, incoming: Incoming) {
        log::debug!("ipc: {}", incoming.request.name());
        let command = match incoming.request {
            Request::Status => return incoming.reply(Response::status(self.status())),
            Request::Move => Command::Move,
            Request::ResetPosition => Command::ResetPosition,
            Request::Reload => Command::Reload,
            Request::Forget => Command::Forget,
            Request::Quit => {
                // Answer first: after Quit the loop is gone.
                incoming.reply(Response::ok());
                return self.handle(surface, Event::Command(Command::Quit));
            }
        };
        self.handle(surface, Event::Command(command));
        let response = if command == Command::Move && !self.badge.is_moving() {
            Response::error("no target display")
        } else {
            Response::ok()
        };
        incoming.reply(response);
    }

    /// The core's next deadline, if any.
    pub fn deadline(&self) -> Option<Instant> {
        self.badge.next_deadline()
    }

    /// How long the native loop may sleep. `None` means until an event.
    pub fn timeout(&self) -> Option<Duration> {
        self.deadline()
            .map(|d| d.saturating_duration_since(Instant::now()))
    }

    /// Call after every wakeup: sends `Deadline` once it has passed, and
    /// reloads the config if the file changed. That check costs one `stat`
    /// per wakeup and needs no watcher thread (SPEC §8.5).
    pub fn tick(&mut self, surface: &mut impl Surface) {
        if self.deadline().is_some_and(|d| d <= Instant::now()) {
            self.handle(surface, Event::Deadline);
        }
        if self.started && !self.preview && modified(&self.paths) != self.config_mtime {
            log::info!("config.toml changed");
            self.handle(surface, Event::Command(Command::Reload));
        }
    }

    /// True once the badge should exit (`pawprint quit`, tray Quit).
    pub fn done(&self) -> bool {
        self.done
    }

    fn run(&mut self, surface: &mut impl Surface, effects: Vec<Effect>) {
        let rest = if self.preview {
            surface::apply(&mut Backdrop(surface), effects)
        } else {
            surface::apply(surface, effects)
        };
        for effect in rest {
            match effect {
                Effect::SaveState(_) if self.preview => {}
                Effect::SaveState(state) => {
                    if let Err(e) = state.save_atomic(&self.paths.state_file) {
                        log::error!("cannot save {}: {e}", self.paths.state_file.display());
                    }
                }
                Effect::ReloadConfig => {
                    self.config_mtime = modified(&self.paths);
                    let (config, note) = commands::load_config(&self.paths);
                    log::info!("reloaded config {note}");
                    config
                        .warnings
                        .iter()
                        .for_each(|w| log::warn!("config: {w}"));
                    self.handle(surface, Event::ConfigTouched(config));
                }
                Effect::SetTray(state) => {
                    if let Some(tray) = &mut self.tray {
                        tray.show(&state);
                    }
                    self.tray_state = Some(state);
                }
                Effect::Exit => {
                    log::info!("quitting");
                    self.done = true;
                }
                Effect::Present(_) | Effect::Place(_) | Effect::SetInteractive(_) => {
                    unreachable!("surface::apply runs surface effects")
                }
            }
        }
    }

    pub fn status(&self) -> Status {
        let r = self.badge.resolved();
        let target = self.badge.target();
        Status {
            pid: std::process::id(),
            backend: self.backend.clone(),
            session: self.session.clone(),
            display: target.map(|d| d.name.clone()),
            rect: self.badge.placed().map(|p| {
                [
                    i64::from(p.x),
                    i64::from(p.y),
                    i64::from(p.width),
                    i64::from(p.height),
                ]
            }),
            scale: target.map(|d| d.scale),
            mascot: r.mascot.clone(),
            label: r.label.clone(),
            position: r.position.value.to_string(),
            position_source: r.position.source.to_string(),
            size: r.settings.size.value,
            moving: self.badge.is_moving(),
        }
    }
}

fn modified(paths: &Paths) -> Option<SystemTime> {
    std::fs::metadata(&paths.config_file)
        .and_then(|m| m.modified())
        .ok()
}

/// The preview's surface adapter: every frame on [`PREVIEW_BACKDROP`] with
/// [`PREVIEW_MARGIN`] around it, and the window grown to match, so each
/// backend's own surface becomes the preview window with no new drawing
/// code.
struct Backdrop<'a, S: Surface>(&'a mut S);

impl<S: Surface> Surface for Backdrop<'_, S> {
    fn displays(&self) -> Vec<DisplayInfo> {
        self.0.displays()
    }

    fn present(&mut self, frame: &Frame) {
        let m = PREVIEW_MARGIN as usize;
        let (w, h) = (frame.width as usize + 2 * m, frame.height as usize + 2 * m);
        let [r, g, b] = PREVIEW_BACKDROP;
        let mut img = pawprint_core::sprite::RgbaImage::new(w as u32, h as u32);
        img.data
            .chunks_mut(4)
            .for_each(|p| p.copy_from_slice(&[r, g, b, 255]));
        // Binary alpha (SPEC §4.5): each pixel is the badge's or the backdrop's.
        for (i, p) in frame.bgra.chunks(4).enumerate() {
            if p[3] != 0 {
                let (x, y) = (i % frame.width as usize + m, i / frame.width as usize + m);
                img.put(x as i64, y as i64, [p[2], p[1], p[0], 255]);
            }
        }
        self.0.present(&Frame::from_rgba(&img));
    }

    fn place(&mut self, rect: PhysicalRect) {
        let m = PREVIEW_MARGIN;
        self.0.place(PhysicalRect::new(
            rect.x - m as i32,
            rect.y - m as i32,
            rect.width + 2 * m,
            rect.height + 2 * m,
        ));
    }

    fn set_interactive(&mut self, on: bool) {
        self.0.set_interactive(on);
    }
}

fn log_displays(displays: &[DisplayInfo]) {
    if displays.is_empty() {
        log::warn!("no displays");
    }
    for d in displays {
        log::info!("display {}", describe(d));
    }
}

/// One display on one line, as `doctor` and the log show it.
pub fn describe(d: &DisplayInfo) -> String {
    let r = |r: pawprint_core::geom::PhysicalRect| {
        format!("{}x{}{:+}{:+}", r.width, r.height, r.x, r.y)
    };
    format!(
        "{} {} scale {} usable {}{}",
        d.name,
        r(d.bounds),
        d.scale,
        r(d.work_area),
        if d.primary { " (primary)" } else { "" }
    )
}
