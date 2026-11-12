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
//!
//! The same shell runs `pawprint preview` and `pawprint settings` in a
//! normal window ([`Shell::is_window`]); backends report that window's
//! pointer and keys through [`Shell::input`].

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use pawprint_core::badge::{Badge, Command, Effect, Event, Inputs};
use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::config::{Animation, Backend, Layer};
use pawprint_core::config_edit;
use pawprint_core::geom::PhysicalRect;
use pawprint_core::layout::Anchor;
use pawprint_core::menu::TrayState;
use pawprint_core::render::Frame;
use pawprint_core::settings;
use pawprint_core::state::State;
use pawprint_core::surface::{self, DisplayInfo, Surface};
use pawprint_core::ui::{Input, UiKey};
use pawprint_core::update::{Job, News};

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

/// What the shell runs. There is one per process, so its size is moot.
#[allow(clippy::large_enum_variant)]
enum Kind {
    /// The badge, or with `preview` its preview window.
    Badge {
        badge: Badge<'static>,
        preview: bool,
    },
    /// The settings window (SPEC §8.7).
    Settings(settings::App),
}

pub struct Shell {
    kind: Kind,
    /// The `backend` setting when the shell started, for backend selection.
    configured: Backend,
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
        let badge = Badge::new(load_catalog(&paths), bundle(), inputs, Instant::now());
        let r = badge.resolved();
        log::info!(
            "mascot {} ({}), label {:?}",
            r.mascot,
            r.trace.last().map_or("", |s| s.outcome.as_str()),
            r.label
        );
        r.warnings.iter().for_each(|w| log::warn!("config: {w}"));
        Ok(Shell {
            configured: r.settings.backend.value,
            kind: Kind::Badge { badge, preview },
            paths,
            session,
            backend: String::new(),
            displays: Vec::new(),
            started: false,
            done: false,
            tray: None,
            tray_state: None,
            config_mtime,
        })
    }

    /// `pawprint settings` (SPEC §8.7): the settings window for this host.
    /// It writes config.toml itself and asks the running badge to reload;
    /// it serves no IPC and shows no tray.
    pub fn settings(paths: Paths) -> Result<Shell, (u8, String)> {
        let hostname = hostname::detect().map_err(|e| (exit::ERROR, e))?;
        let config_mtime = modified(&paths);
        let text = read_config(&paths).map_err(|e| (exit::ERROR, e))?;
        let (state, _) = State::load(&paths.state_file);
        let configured = pawprint_core::config::Config::parse(&text)
            .effective(
                &pawprint_core::host::short_name(&hostname),
                &Layer::default(),
            )
            .backend
            .value;
        let inputs = settings::Inputs {
            hostname,
            text,
            state,
            path: paths.config_file.display().to_string(),
        };
        let app = settings::App::new(load_catalog(&paths), bundle(), inputs);
        Ok(Shell {
            kind: Kind::Settings(app),
            configured,
            paths,
            session: "settings".into(),
            backend: String::new(),
            displays: Vec::new(),
            started: false,
            done: false,
            tray: None,
            tray_state: None,
            config_mtime,
        })
    }

    /// True for `pawprint preview` and `pawprint settings`: backends open
    /// a normal, focusable, closable window instead of the overlay, and
    /// report its input through [`Shell::input`].
    pub fn is_window(&self) -> bool {
        !matches!(self.kind, Kind::Badge { preview: false, .. })
    }

    /// The normal window's title.
    pub fn title(&self) -> &'static str {
        match self.kind {
            Kind::Settings(_) => "PAWPRINT settings",
            _ => "PAWPRINT preview",
        }
    }

    fn previewing(&self) -> bool {
        matches!(self.kind, Kind::Badge { preview: true, .. })
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
    pub fn configured_backend(&self) -> Backend {
        self.configured
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
        self.started = true;
        match &mut self.kind {
            Kind::Badge { badge, .. } => {
                let effects = badge.start(self.displays.clone(), clock::now());
                self.run(surface, effects);
            }
            Kind::Settings(app) => {
                let effects = app.start(self.displays.clone());
                self.run_settings(surface, effects);
            }
        }
    }

    /// Input to the normal window ([`Shell::is_window`]). The preview
    /// closes on Esc (or Ctrl+W, Ctrl+Q); the settings window takes it all.
    pub fn input(&mut self, surface: &mut impl Surface, input: Input) {
        if !self.started || self.done {
            return;
        }
        match &mut self.kind {
            Kind::Settings(app) => {
                let effects = app.input(input);
                self.run_settings(surface, effects);
            }
            Kind::Badge { preview: true, .. } => {
                if matches!(
                    input,
                    Input::Key(UiKey::Escape) | Input::Close | Input::Shortcut('w' | 'q')
                ) {
                    self.handle(surface, Event::Command(Command::Quit));
                }
            }
            Kind::Badge { .. } => {}
        }
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
        let Kind::Badge { badge, preview } = &self.kind else {
            return;
        };
        let short = badge.resolved().short.clone();
        if let Some((key, value)) = edit.filter(|_| !*preview) {
            match commands::edit_config(&self.paths, |text| {
                config_edit::set(text, &short, key, &value)
            }) {
                Ok(()) => {
                    log::info!("saved {key} = {value} in [hosts.{short:?}]");
                    return self.handle(surface, Event::Command(Command::Reload));
                }
                Err((_, e)) => {
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
        match &mut self.kind {
            Kind::Badge { badge, .. } => {
                let effects = badge.handle(event, clock::now());
                self.run(surface, effects);
            }
            Kind::Settings(app) => match event {
                Event::DisplaysChanged(displays) => {
                    let effects = app.displays_changed(displays);
                    self.run_settings(surface, effects);
                }
                // The window's close button: finish typing, then close.
                Event::Command(Command::Quit) => self.input(surface, Input::Close),
                _ => {}
            },
        }
    }

    /// Answer one IPC request.
    pub fn request(&mut self, surface: &mut impl Surface, incoming: Incoming) {
        log::debug!("ipc: {}", incoming.request.name());
        if !matches!(self.kind, Kind::Badge { .. }) {
            return incoming.reply(Response::error("this is the settings window"));
        }
        let command = match incoming.request {
            Request::Status => return incoming.reply(Response::status(self.status())),
            Request::Move => Command::Move,
            Request::ResetPosition => Command::ResetPosition,
            Request::Reload => Command::Reload,
            Request::Forget => Command::Forget,
            Request::Update(news) => Command::UpdateNews(news),
            Request::Quit => {
                // Answer first: after Quit the loop is gone.
                incoming.reply(Response::ok());
                return self.handle(surface, Event::Command(Command::Quit));
            }
        };
        self.handle(surface, Event::Command(command));
        let moving = matches!(&self.kind, Kind::Badge { badge, .. } if badge.is_moving());
        let response = if command == Command::Move && !moving {
            Response::error("no target display")
        } else {
            Response::ok()
        };
        incoming.reply(response);
    }

    /// The display the core put the badge on (SPEC §6.2), for a backend
    /// whose surfaces are tied to an output (Wayland), so it never picks
    /// one itself.
    #[cfg(target_os = "linux")]
    pub fn target(&self) -> Option<&DisplayInfo> {
        match &self.kind {
            Kind::Badge { badge, .. } => badge.target(),
            Kind::Settings(app) => app.target(),
        }
    }

    /// The core's next deadline, if any.
    pub fn deadline(&self) -> Option<Instant> {
        match &self.kind {
            Kind::Badge { badge, .. } => badge.next_deadline(),
            Kind::Settings(_) => None,
        }
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
        if self.started && !self.previewing() && modified(&self.paths) != self.config_mtime {
            log::info!("config.toml changed");
            self.config_mtime = modified(&self.paths);
            match &mut self.kind {
                Kind::Settings(app) => {
                    if let Ok(text) = read_config(&self.paths) {
                        let effects = app.reload(text);
                        self.run_settings(surface, effects);
                    }
                }
                Kind::Badge { .. } => self.handle(surface, Event::Command(Command::Reload)),
            }
        }
    }

    /// True once the badge should exit (`pawprint quit`, tray Quit).
    pub fn done(&self) -> bool {
        self.done
    }

    /// Execute the settings window's effects (SPEC §8.7).
    fn run_settings(&mut self, surface: &mut impl Surface, effects: Vec<settings::Effect>) {
        for effect in effects {
            let more = match effect {
                settings::Effect::Present(frame) => {
                    surface.present(&frame);
                    continue;
                }
                settings::Effect::Place(rect) => {
                    surface.place(rect);
                    continue;
                }
                settings::Effect::Close => {
                    self.done = true;
                    continue;
                }
                settings::Effect::Write(text) => {
                    let result = commands::write_config(&self.paths.config_file, &text);
                    self.config_mtime = modified(&self.paths);
                    match result {
                        Ok(()) => {
                            log::info!("settings: saved {}", self.paths.config_file.display());
                            commands::reload_running();
                            continue;
                        }
                        Err((_, e)) => self.settings_app().map(|app| app.failed(&e)),
                    }
                }
                settings::Effect::ResetPosition => {
                    commands::reset_position(&self.paths);
                    let (state, _) = State::load(&self.paths.state_file);
                    self.settings_app().map(|app| app.set_state(state))
                }
            };
            if let Some(more) = more {
                self.run_settings(surface, more);
            }
        }
    }

    fn settings_app(&mut self) -> Option<&mut settings::App> {
        match &mut self.kind {
            Kind::Settings(app) => Some(app),
            Kind::Badge { .. } => None,
        }
    }

    fn run(&mut self, surface: &mut impl Surface, effects: Vec<Effect>) {
        let preview = self.previewing();
        let rest = if preview {
            surface::apply(&mut Backdrop(surface), effects)
        } else {
            surface::apply(surface, effects)
        };
        for effect in rest {
            match effect {
                Effect::SaveState(_) if preview => {}
                Effect::SaveState(state) => {
                    if let Err(e) = state.save_atomic(&self.paths.state_file) {
                        log::error!("cannot save {}: {e}", self.paths.state_file.display());
                    }
                }
                Effect::ReloadConfig => {
                    // User mascots too, so `set mascot <new folder>` works.
                    let catalog = load_catalog(&self.paths);
                    let Kind::Badge { badge, .. } = &mut self.kind else {
                        unreachable!("only the badge reloads")
                    };
                    let effects = badge.set_catalog(catalog, clock::now());
                    self.run(surface, effects);
                    self.config_mtime = modified(&self.paths);
                    let (config, note) = commands::load_config(&self.paths);
                    log::info!("reloaded config {note}");
                    config
                        .warnings
                        .iter()
                        .for_each(|w| log::warn!("config: {w}"));
                    self.handle(surface, Event::ConfigTouched(config));
                }
                Effect::OpenSettings if preview => {}
                Effect::OpenSettings => {
                    if let Err(e) = crate::instance::spawn_detached(&["settings"]) {
                        log::error!("cannot open the settings window: {e}");
                    }
                }
                Effect::Update(_) if preview => {}
                Effect::Update(job) => {
                    // It reports back over IPC (SPEC §15.8).
                    let args: &[&str] = match job {
                        Job::Check => &["update", "--check"],
                        Job::Install => &["update"],
                    };
                    if let Err(e) = crate::instance::spawn_detached(args) {
                        log::error!("cannot run pawprint {}: {e}", args.join(" "));
                        self.handle(surface, Event::Command(Command::UpdateNews(News::Failed)));
                    }
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
                Effect::Present(_)
                | Effect::Place(_)
                | Effect::SetInteractive(_)
                | Effect::SetFullscreen(_) => {
                    unreachable!("surface::apply runs surface effects")
                }
            }
        }
    }

    pub fn status(&self) -> Status {
        let Kind::Badge { badge, .. } = &self.kind else {
            unreachable!("only the badge serves IPC")
        };
        let r = badge.resolved();
        let target = badge.target();
        Status {
            pid: std::process::id(),
            backend: self.backend.clone(),
            session: self.session.clone(),
            display: target.map(|d| d.name.clone()),
            rect: badge.placed().map(|p| {
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
            moving: badge.is_moving(),
        }
    }
}

/// Bundled plus user mascots, read now (SPEC §12.7); invalid ones are
/// left out with a warning in the log.
fn load_catalog(paths: &Paths) -> Arc<Catalog> {
    let catalog = Catalog::load(&paths.user_mascots);
    catalog.warnings().iter().for_each(|w| log::warn!("{w}"));
    Arc::new(catalog)
}

/// config.toml's text, empty if there is none.
fn read_config(paths: &Paths) -> Result<String, String> {
    match std::fs::read_to_string(&paths.config_file) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("cannot read {}: {e}", paths.config_file.display())),
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

    /// A preview is a normal window: fullscreen windows treat it like any.
    fn set_fullscreen(&mut self, _: pawprint_core::config::Fullscreen) {}
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
