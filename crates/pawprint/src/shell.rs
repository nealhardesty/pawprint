//! The imperative shell around `pawprint_core::badge::Badge` (SPEC §10.1),
//! written once for every backend.
//!
//! A backend owns its native loop and a [`Surface`]. It calls
//! [`Shell::start`] once, feeds translated native events to
//! [`Shell::handle`], hands IPC requests to [`Shell::request`], sleeps for at
//! most [`Shell::timeout`], calls [`Shell::tick`] after every wakeup, and
//! stops when [`Shell::done`] is true. Everything else (state saving, config
//! reloads, status, logging) happens here.

use std::time::{Duration, Instant};

use pawprint_core::badge::{Badge, Command, Effect, Event, Inputs};
use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::config::Layer;
use pawprint_core::state::State;
use pawprint_core::surface::{self, DisplayInfo, Surface};

use crate::ipc::{Incoming, Request, Response, Status};
use crate::paths::Paths;
use crate::{clock, commands, exit, hostname};

pub struct Shell {
    badge: Badge<'static>,
    paths: Paths,
    session: String,
    backend: String,
    /// The display set last given to the core, to drop repeats.
    displays: Vec<DisplayInfo>,
    started: bool,
    done: bool,
}

impl Shell {
    pub fn new(paths: Paths, session: String, flags: Layer) -> Result<Shell, (u8, String)> {
        let hostname = hostname::detect().map_err(|e| (exit::ERROR, e))?;
        let (config, config_note) = commands::load_config(&paths);
        let (state, warning) = State::load(&paths.state_file);
        log::info!("hostname {hostname:?}, config {config_note}");
        warning.into_iter().for_each(|w| log::warn!("{w}"));
        let inputs = Inputs {
            hostname,
            config,
            state,
            flags,
            seed: clock::seed(),
        };
        let badge = Badge::new(Catalog::bundled(), bundle(), inputs, Instant::now());
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
        })
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

    /// Call after every wakeup: sends `Deadline` once it has passed.
    pub fn tick(&mut self, surface: &mut impl Surface) {
        if self.deadline().is_some_and(|d| d <= Instant::now()) {
            self.handle(surface, Event::Deadline);
        }
    }

    /// True once the badge should exit (`pawprint quit`, tray Quit).
    pub fn done(&self) -> bool {
        self.done
    }

    fn run(&mut self, surface: &mut impl Surface, effects: Vec<Effect>) {
        for effect in surface::apply(surface, effects) {
            match effect {
                Effect::SaveState(state) => {
                    if let Err(e) = state.save_atomic(&self.paths.state_file) {
                        log::error!("cannot save {}: {e}", self.paths.state_file.display());
                    }
                }
                Effect::ReloadConfig => {
                    let (config, note) = commands::load_config(&self.paths);
                    log::info!("reloaded config {note}");
                    config
                        .warnings
                        .iter()
                        .for_each(|w| log::warn!("config: {w}"));
                    self.handle(surface, Event::ConfigTouched(config));
                }
                // The tray arrives in M4 (ROADMAP); the core already models it.
                Effect::SetTray(_) => {}
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
