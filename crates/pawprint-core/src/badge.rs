//! The badge as a pure state machine (SPEC §10.1): backends feed it
//! [`Event`]s and execute the [`Effect`]s it returns, then sleep until
//! [`Badge::next_deadline`] or the next native event.

use std::sync::Arc;
use std::time::Instant;

use pawprint_sprite::{Bundle, Mascot, Rgb, RgbaImage};

use crate::anim::Scheduler;
use crate::catalog::Catalog;
use crate::config::{Animation, Config, Fullscreen, Layer};
use crate::geom::{PhysicalRect, Point};
use crate::identity::{self, Resolved};
use crate::layout::{self, LabelSize};
use crate::menu::{self, TrayState};
use crate::movemode::{self, Key, MoveMode, Outcome};
use crate::render::{self, BadgeSpec, Frame};
use crate::state::State;
use crate::surface::{DisplayInfo, select_display};
use crate::update::{Job, News, Notice, Version};

/// The current time, as the shell sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    pub instant: Instant,
    /// Local clock hour, 0–23, for `when = "day" | "night"`.
    pub local_hour: u8,
}

/// Commands from IPC (`pawprint move`, ...) or the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Move,
    ResetPosition,
    Reload,
    Quit,
    SetSize(u32),
    SetAnimation(Animation),
    /// Drop this host's cached fallback mascot (`pawprint forget`).
    Forget,
    /// Open the settings window (the tray's Settings…).
    Settings,
    /// Ask what the latest release is (the tray's Check for updates).
    CheckUpdate,
    /// Install the release a check found (the tray's Install update).
    InstallUpdate,
    /// How a check or an install ended (`pawprint update`, over IPC).
    UpdateNews(News),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The display set, a resolution, a scale, or a work area changed. Also
    /// sent once at startup.
    DisplaysChanged(Vec<DisplayInfo>),
    /// `next_deadline()` was reached.
    Deadline,
    /// Pointer pressed, moved, or released over the badge, in physical
    /// desktop coordinates. Only meaningful in move mode.
    DragStart(Point),
    DragMove(Point),
    DragEnd(Point),
    Key(Key),
    Command(Command),
    /// The config file changed and the shell re-read it.
    ConfigTouched(Config),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Present(Frame),
    Place(PhysicalRect),
    SetInteractive(bool),
    /// Step aside for fullscreen windows, or stay on top of them (SPEC §6.5).
    SetFullscreen(Fullscreen),
    SetTray(TrayState),
    /// Write this to the state file atomically.
    SaveState(State),
    /// Re-read the config file and send `Event::ConfigTouched`.
    ReloadConfig,
    /// Start the settings window in a process of its own (SPEC §8.7).
    OpenSettings,
    /// Run this in a process of its own (`pawprint update`); it answers
    /// with `Command::UpdateNews` (SPEC §15.8).
    Update(Job),
    Exit,
}

/// What a badge is built from. The shell reads these from the OS and disk.
#[derive(Debug, Clone)]
pub struct Inputs {
    pub hostname: String,
    pub config: Config,
    pub state: State,
    /// `pawprint run` flag overrides.
    pub flags: Layer,
    /// Seeds animation timing; the shell uses the clock.
    pub seed: u64,
    /// `pawprint preview`: play every action in turn.
    pub preview: bool,
}

/// Everything the presented image depends on: when it matches, the badge is
/// not recomposed (SPEC §4.5).
#[derive(Debug, Clone, PartialEq)]
struct RenderKey {
    mascot: String,
    frame: usize,
    label: Option<String>,
    accent: Rgb,
    size: u32,
    label_size: LabelSize,
    scale: f64,
    moving: bool,
}

pub struct Badge<'c> {
    /// Owned, so a reload can swap in a catalog with the user mascots
    /// re-read ([`Badge::set_catalog`]).
    catalog: Arc<Catalog>,
    assets: &'c Bundle,
    hostname: String,
    config: Config,
    flags: Layer,
    state: State,
    resolved: Resolved,
    displays: Vec<DisplayInfo>,
    anim: Scheduler,
    moving: Option<MoveMode>,
    placed: Option<PhysicalRect>,
    presented: Option<(RenderKey, RgbaImage)>,
    tray: Option<TrayState>,
    /// The fullscreen policy last sent to the surface.
    fullscreen: Option<Fullscreen>,
    /// The tray icon's mascot and accent, so it is composed only on change.
    tray_icon: Option<(String, Rgb)>,
    /// What the crate says about an update instead of the hostname, and
    /// until when (SPEC §15.8).
    notice: Option<(Notice, Instant)>,
    /// The newer release a check found; the tray offers to install it.
    available: Option<Version>,
    /// How many times the badge image was composed (for tests).
    compositions: u32,
}

impl<'c> Badge<'c> {
    pub fn new(catalog: Arc<Catalog>, assets: &'c Bundle, inputs: Inputs, now: Instant) -> Self {
        let Inputs {
            hostname,
            config,
            state,
            flags,
            seed,
            preview,
        } = inputs;
        let resolved = identity::resolve(
            &hostname,
            &config,
            &state,
            &flags,
            &catalog,
            &assets.accents,
            false,
        );
        let mut anim = Scheduler::new(
            resolved.settings.animation.value,
            resolved.settings.interval.value,
            seed,
            now,
        );
        if preview {
            anim.cycle_all(now);
        }
        Badge {
            catalog,
            assets,
            hostname,
            config,
            flags,
            state,
            resolved,
            displays: Vec::new(),
            anim,
            moving: None,
            placed: None,
            presented: None,
            tray: None,
            fullscreen: None,
            tray_icon: None,
            notice: None,
            available: None,
            compositions: 0,
        }
    }

    pub fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    pub fn is_moving(&self) -> bool {
        self.moving.is_some()
    }

    /// Where the badge was last placed, in physical desktop pixels.
    pub fn placed(&self) -> Option<PhysicalRect> {
        self.placed
    }

    /// The display the badge is on (SPEC §6.2).
    pub fn target(&self) -> Option<&DisplayInfo> {
        select_display(&self.displays, &self.resolved.settings.display.value)
    }

    /// Use this catalog from now on (the shell re-read the user mascots on
    /// a reload), and show whatever that changes. A mascot that left the
    /// catalog is resolved away at once.
    pub fn set_catalog(&mut self, catalog: Arc<Catalog>, now: Now) -> Vec<Effect> {
        let mut out = Vec::new();
        self.catalog = catalog;
        self.reresolve(now, &mut out);
        self.refresh(&mut out);
        out
    }

    /// First call after `new`: show the badge.
    pub fn start(&mut self, displays: Vec<DisplayInfo>, now: Now) -> Vec<Effect> {
        let mut out = vec![Effect::SetInteractive(false)];
        self.displays = displays;
        self.reresolve(now, &mut out);
        self.refresh(&mut out);
        out
    }

    /// When to wake up next if no native event arrives first. `None` means
    /// sleep until an event (static mode, not moving).
    pub fn next_deadline(&self) -> Option<Instant> {
        let anim = self.anim.next_deadline();
        let moving = self.moving.as_ref().map(MoveMode::deadline);
        let notice = self.notice.map(|(_, until)| until);
        anim.into_iter().chain(moving).chain(notice).min()
    }

    pub fn handle(&mut self, event: Event, now: Now) -> Vec<Effect> {
        let mut out = Vec::new();
        match event {
            Event::DisplaysChanged(displays) => {
                self.displays = displays;
                if self.moving.take().is_some() {
                    out.push(Effect::SetInteractive(false));
                }
            }
            Event::Deadline => {
                if let Some(o) = self.moving.as_mut().and_then(|m| m.timeout(now.instant)) {
                    self.finish_move(o, now, &mut out);
                }
                if let Some((notice, until)) = self.notice
                    && until <= now.instant
                {
                    // A job that never answered has failed.
                    self.notice = None;
                    if notice.busy() {
                        self.show_notice(Notice::Failed, now);
                    }
                }
                let mascot = self
                    .catalog
                    .get(&self.resolved.mascot)
                    .expect("resolved mascots exist");
                self.anim.on_deadline(now.instant, now.local_hour, mascot);
            }
            Event::DragStart(p) => {
                if let Some(m) = self.moving.as_mut() {
                    m.drag_start(p, now.instant);
                }
            }
            Event::DragMove(p) | Event::DragEnd(p) => {
                let usable = self.target().map(|d| d.work_area);
                if let (Some(m), Some(usable)) = (self.moving.as_mut(), usable) {
                    let o = if matches!(event, Event::DragEnd(_)) {
                        m.drag_end(p, usable, now.instant)
                    } else {
                        m.drag_move(p, usable, now.instant)
                    };
                    self.finish_move(o, now, &mut out);
                }
            }
            Event::Key(key) => {
                let target = self.target().map(|d| (d.work_area, d.scale));
                if let (Some(m), Some((usable, scale))) = (self.moving.as_mut(), target) {
                    let o = m.key(key, usable, scale, now.instant);
                    self.finish_move(o, now, &mut out);
                }
            }
            Event::Command(cmd) => self.command(cmd, now, &mut out),
            Event::ConfigTouched(config) => {
                self.config = config;
                self.reresolve(now, &mut out);
            }
        }
        self.refresh(&mut out);
        out
    }

    fn command(&mut self, cmd: Command, now: Now, out: &mut Vec<Effect>) {
        match cmd {
            Command::Move => {
                if let (None, Some(rect)) = (&self.moving, self.placed) {
                    self.moving = Some(MoveMode::new(rect, now.instant));
                    out.push(Effect::SetInteractive(true));
                }
            }
            Command::ResetPosition => {
                self.state.position = None;
                out.push(Effect::SaveState(self.state.clone()));
                self.reresolve(now, out);
            }
            Command::Reload => out.push(Effect::ReloadConfig),
            Command::Settings => out.push(Effect::OpenSettings),
            Command::Quit => out.push(Effect::Exit),
            Command::SetSize(size) => {
                self.flags.size = Some(size);
                self.reresolve(now, out);
            }
            Command::SetAnimation(mode) => {
                self.flags.animation = Some(mode);
                self.reresolve(now, out);
            }
            Command::CheckUpdate | Command::InstallUpdate if self.updating() => {}
            Command::CheckUpdate => {
                self.show_notice(Notice::Checking, now);
                out.push(Effect::Update(Job::Check));
            }
            Command::InstallUpdate => {
                if self.available.is_some() {
                    self.show_notice(Notice::Updating, now);
                    out.push(Effect::Update(Job::Install));
                }
            }
            Command::UpdateNews(news) => {
                self.available = match news {
                    News::Available(v) => Some(v),
                    News::UpToDate | News::Updated => None,
                    News::Failed => self.available,
                };
                self.show_notice(news.into(), now);
            }
            Command::Forget => {
                if self
                    .state
                    .assignments
                    .remove(&self.resolved.short)
                    .is_some()
                {
                    let n = out.len();
                    self.reresolve(now, out);
                    if !out[n..].iter().any(|e| matches!(e, Effect::SaveState(_))) {
                        out.push(Effect::SaveState(self.state.clone()));
                    }
                }
            }
        }
    }

    /// A check or an install is running.
    fn updating(&self) -> bool {
        self.notice.is_some_and(|(n, _)| n.busy())
    }

    /// Say `notice` on the crate for as long as it lasts; good news gets
    /// the mascot's signature action too.
    fn show_notice(&mut self, notice: Notice, now: Now) {
        self.notice = Some((notice, now.instant + notice.duration()));
        if notice.happy()
            && let Some(mascot) = self.catalog.get(&self.resolved.mascot)
        {
            self.anim.celebrate(now.instant, mascot);
        }
    }

    fn finish_move(&mut self, outcome: Outcome, now: Now, out: &mut Vec<Effect>) {
        let Outcome::Done { rect, save } = outcome else {
            return;
        };
        self.moving = None;
        out.push(Effect::SetInteractive(false));
        if save && let Some(d) = self.target() {
            self.state.position = Some(layout::snap(rect, d.work_area, d.scale));
            out.push(Effect::SaveState(self.state.clone()));
            self.reresolve(now, out);
        }
    }

    /// Recompute identity and settings after anything they depend on changed.
    fn reresolve(&mut self, now: Now, out: &mut Vec<Effect>) {
        let before = self.resolved.mascot.clone();
        self.resolved = identity::resolve(
            &self.hostname,
            &self.config,
            &self.state,
            &self.flags,
            &self.catalog,
            &self.assets.accents,
            false,
        );
        if let Some(id) = self.resolved.new_assignment.take() {
            self.state
                .assignments
                .insert(self.resolved.short.clone(), id);
            out.push(Effect::SaveState(self.state.clone()));
        }
        let s = &self.resolved.settings;
        if self.resolved.mascot != before {
            self.anim.restart(now.instant);
        }
        self.anim
            .configure(s.animation.value, s.interval.value, now.instant);
    }

    /// Emit Place / Present / SetTray only for what actually changed.
    fn refresh(&mut self, out: &mut Vec<Effect>) {
        let catalog = Arc::clone(&self.catalog);
        let mascot: &Mascot = catalog
            .get(&self.resolved.mascot)
            .expect("resolved mascots exist");
        let icon_key = (self.resolved.mascot.clone(), self.resolved.accent.rgb);
        let icon = match &self.tray {
            Some(t) if self.tray_icon.as_ref() == Some(&icon_key) => t.icon.clone(),
            _ => render::compose_1x(&self.assets.crate_art, mascot, 0, icon_key.1),
        };
        self.tray_icon = Some(icon_key);
        let tray = TrayState {
            icon,
            visible: self.resolved.settings.tray.value,
            tooltip: format!(
                "PAWPRINT: {} ({})",
                self.resolved
                    .label
                    .as_deref()
                    .unwrap_or(&self.resolved.short),
                self.resolved.mascot
            ),
            menu: menu::menu(
                self.resolved.settings.size.value,
                self.resolved.settings.animation.value,
                self.moving.is_some(),
                self.notice.map(|(n, _)| n),
                self.available,
            ),
        };
        if self.tray.as_ref() != Some(&tray) {
            self.tray = Some(tray.clone());
            out.push(Effect::SetTray(tray));
        }

        let policy = self.resolved.settings.fullscreen.value;
        if self.fullscreen != Some(policy) {
            self.fullscreen = Some(policy);
            out.push(Effect::SetFullscreen(policy));
        }

        let Some((work_area, scale)) = self.target().map(|d| (d.work_area, d.scale)) else {
            return;
        };
        let s = &self.resolved.settings;
        let key = RenderKey {
            mascot: self.resolved.mascot.clone(),
            frame: self.anim.frame(mascot),
            label: match self.notice {
                Some((notice, _)) => Some(notice.label()),
                None => self.resolved.label.clone(),
            },
            accent: self.resolved.accent.rgb,
            size: s.size.value,
            label_size: s.label_size.value,
            scale,
            moving: self.moving.is_some(),
        };
        let changed = self.presented.as_ref().map(|(k, _)| k) != Some(&key);
        let mut present = None;
        let (width, height) = match &self.presented {
            Some((_, img)) if !changed => (img.width, img.height),
            _ => {
                let spec = BadgeSpec {
                    mascot,
                    frame: key.frame,
                    label: key.label.as_deref(),
                    accent: key.accent,
                    size: key.size,
                    label_size: key.label_size,
                    display_scale: key.scale,
                };
                let mut img = render::render_badge(&spec, self.assets);
                if key.moving {
                    let p = layout::art_scale(key.size, key.scale);
                    movemode::draw_outline(&mut img, key.accent, p);
                }
                self.compositions += 1;
                let size = (img.width, img.height);
                if self.presented.as_ref().is_none_or(|(_, old)| *old != img) {
                    present = Some(Frame::from_rgba(&img));
                }
                self.presented = Some((key, img));
                size
            }
        };
        let rect = match &self.moving {
            Some(m) => PhysicalRect {
                width,
                height,
                ..m.rect()
            },
            None => layout::place(
                self.resolved.position.value,
                width,
                height,
                work_area,
                scale,
            ),
        };
        if self.placed != Some(rect) {
            self.placed = Some(rect);
            out.push(Effect::Place(rect));
        }
        out.extend(present.map(Effect::Present));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::catalog::bundle;
    use crate::config::Source;
    use crate::layout::{Anchor, Position};
    use crate::movemode::KeyCode;
    use crate::surface::{FakeSurface, Surface, apply};

    fn display(name: &str, w: u32, h: u32, primary: bool) -> DisplayInfo {
        DisplayInfo {
            name: name.into(),
            bounds: PhysicalRect::new(0, 0, w, h),
            work_area: PhysicalRect::new(0, 0, w, h - 40),
            scale: 1.0,
            primary,
        }
    }

    /// A badge driven through a FakeSurface, the way a backend drives it.
    struct Rig {
        badge: Badge<'static>,
        surface: FakeSurface,
        shell: Vec<Effect>,
        now: Now,
    }

    impl Rig {
        fn new(host: &str, config: &str) -> Rig {
            Rig::with_state(host, config, State::default())
        }

        fn with_state(host: &str, config: &str, state: State) -> Rig {
            let now = Now {
                instant: Instant::now(),
                local_hour: 12,
            };
            let inputs = Inputs {
                hostname: host.into(),
                config: Config::parse(config),
                state,
                flags: Layer::default(),
                seed: 42,
                preview: false,
            };
            let catalog = Arc::new(Catalog::bundled().clone());
            let mut badge = Badge::new(catalog, bundle(), inputs, now.instant);
            let mut surface = FakeSurface::new(vec![display("DP-1", 1920, 1080, true)]);
            let effects = badge.start(surface.displays(), now);
            let shell = apply(&mut surface, effects);
            Rig {
                badge,
                surface,
                shell,
                now,
            }
        }

        fn send(&mut self, event: Event) -> Vec<Effect> {
            let fx = self.badge.handle(event, self.now);
            let shell = apply(&mut self.surface, fx);
            self.shell.extend(shell.clone());
            shell
        }

        fn advance_to_deadline(&mut self) {
            self.now.instant = self.badge.next_deadline().expect("a deadline");
            self.send(Event::Deadline);
        }
    }

    #[test]
    fn starts_top_right_click_through_with_tray() {
        let rig = Rig::new("owl", "");
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1920 - 64 - 16, 16, 64, 64))
        );
        assert_eq!(rig.surface.frames.len(), 1);
        assert_eq!(
            (rig.surface.frames[0].width, rig.surface.frames[0].height),
            (64, 64)
        );
        assert!(!rig.surface.interactive);
        assert!(matches!(&rig.shell[0], Effect::SetTray(t) if t.tooltip == "PAWPRINT: OWL (owl)"));
        assert!(
            !rig.shell.iter().any(|e| matches!(e, Effect::SaveState(_))),
            "matched hosts write no state"
        );
    }

    #[test]
    fn idle_animation_presents_only_changes_and_never_moves() {
        let mut rig = Rig::new("owl", "interval = [1, 1]");
        let mut presents = 0;
        let mut rest_again = false;
        for _ in 0..40 {
            let before = rig.surface.frames.len();
            rig.advance_to_deadline();
            presents += rig.surface.frames.len() - before;
            rest_again |= presents > 0 && rig.surface.frames.last() == rig.surface.frames.first();
        }
        assert!(presents >= 2, "actions played ({presents} presents)");
        assert!(rest_again, "returned to the rest frame");
        assert_eq!(
            rig.surface.places.len(),
            1,
            "animation never re-places the badge"
        );
    }

    #[test]
    fn static_mode_sleeps_until_an_event() {
        let rig = Rig::new("owl", "animation = \"static\"");
        assert_eq!(rig.badge.next_deadline(), None);
    }

    #[test]
    fn unchanged_badge_is_not_recomposed() {
        let mut rig = Rig::new("owl", "animation = \"static\"");
        assert_eq!(rig.badge.compositions, 1);
        rig.send(Event::Deadline);
        rig.send(Event::Command(Command::Reload));
        assert_eq!(rig.badge.compositions, 1, "nothing changed");
        assert_eq!(rig.surface.frames.len(), 1);

        rig.send(Event::Command(Command::Move));
        assert_eq!(rig.badge.compositions, 2, "the move outline is drawn");
        rig.send(Event::Command(Command::SetSize(128)));
        assert_eq!(rig.badge.compositions, 3, "a new size is drawn");
        assert_eq!(rig.surface.frames.len(), 3);
    }

    #[test]
    fn display_changes_relayout_and_follow_the_primary() {
        let mut rig = Rig::new("owl", "");
        rig.send(Event::DisplaysChanged(vec![display(
            "DP-1", 1280, 720, true,
        )]));
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1280 - 80, 16, 64, 64))
        );

        let mut second = display("HDMI-1", 2560, 1440, true);
        second.bounds.x = 1280;
        second.work_area = PhysicalRect::new(1280, 0, 2560, 1400);
        let mut old = display("DP-1", 1280, 720, false);
        old.scale = 1.0;
        rig.send(Event::DisplaysChanged(vec![old, second]));
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1280 + 2560 - 80, 16, 64, 64))
        );

        // A 2x display doubles everything.
        let mut hidpi = display("DP-1", 3840, 2160, true);
        hidpi.scale = 2.0;
        rig.send(Event::DisplaysChanged(vec![hidpi]));
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(3840 - 128 - 32, 32, 128, 128))
        );
        assert_eq!(rig.surface.frames.last().unwrap().width, 128);
    }

    #[test]
    fn move_mode_drag_snaps_and_saves() {
        let mut rig = Rig::new("owl", "");
        rig.send(Event::Command(Command::Move));
        assert!(rig.surface.interactive);
        assert!(rig.badge.next_deadline().unwrap() <= rig.now.instant + crate::movemode::TIMEOUT);

        rig.send(Event::DragStart(Point::new(1870, 40)));
        rig.send(Event::DragMove(Point::new(100, 1000)));
        assert_eq!(rig.surface.rect(), Some(PhysicalRect::new(70, 976, 64, 64)));
        let shell = rig.send(Event::DragEnd(Point::new(40, 1000)));
        assert!(!rig.surface.interactive);
        let saved = shell.iter().find_map(|e| {
            if let Effect::SaveState(s) = e {
                s.position
            } else {
                None
            }
        });
        // Dropped at (10, 976) in a 1920x1040 work area: bottom-left ninth.
        assert_eq!(
            saved,
            Some(Position {
                anchor: Anchor::BottomLeft,
                offset: [10, 0]
            })
        );
        assert_eq!(rig.badge.resolved().position.source, Source::StateFile);

        // The corner sticks across a resolution change.
        rig.send(Event::DisplaysChanged(vec![display(
            "DP-1", 1280, 720, true,
        )]));
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(10, 680 - 64, 64, 64))
        );

        rig.send(Event::Command(Command::ResetPosition));
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1280 - 80, 16, 64, 64))
        );
        let last_save = rig.shell.iter().rev().find_map(|e| {
            if let Effect::SaveState(s) = e {
                Some(s)
            } else {
                None
            }
        });
        assert_eq!(last_save.unwrap().position, None);
    }

    #[test]
    fn move_mode_shows_the_dashed_accent_outline() {
        let mut rig = Rig::new("owl", "");
        let resting = rig.surface.frames.last().unwrap().clone();
        let accent = rig.badge.resolved().accent.rgb;
        rig.send(Event::Command(Command::Move));
        let moving = rig.surface.frames.last().unwrap().clone();
        assert_ne!(moving, resting, "a new frame with the outline");
        // The outline is drawn over the badge's edge, never around it.
        assert_eq!(
            (moving.width, moving.height),
            (resting.width, resting.height)
        );
        // Top-left pixel: premultiplied BGRA of the accent.
        assert_eq!(&moving.bgra[..4], &[accent.2, accent.1, accent.0, 255]);
        rig.send(Event::Key(Key {
            code: KeyCode::Escape,
            shift: false,
        }));
        assert_eq!(rig.surface.frames.last(), Some(&resting), "gone afterwards");
    }

    #[test]
    fn forget_drops_the_cached_mascot_and_picks_again() {
        let mut state = State::default();
        state.assignments.insert("buildbox".into(), "moose".into());
        let mut rig = Rig::with_state("buildbox", "", state);
        assert_eq!(rig.badge.resolved().mascot, "moose");
        let shell = rig.send(Event::Command(Command::Forget));
        let saved = shell.iter().rev().find_map(|e| match e {
            Effect::SaveState(s) => Some(s.clone()),
            _ => None,
        });
        let saved = saved.expect("state saved");
        assert_eq!(
            saved.assignments.get("buildbox"),
            Some(&rig.badge.resolved().mascot),
            "the new pick is cached"
        );
        // A host matched by name has nothing cached: nothing to save.
        let mut owl = Rig::new("owl", "");
        assert!(
            !owl.send(Event::Command(Command::Forget))
                .iter()
                .any(|e| matches!(e, Effect::SaveState(_)))
        );
    }

    #[test]
    fn tray_icon_is_the_rest_frame_in_its_crate() {
        let rig = Rig::new("owl", "");
        let Some(Effect::SetTray(t)) = rig.shell.first() else {
            panic!("tray first")
        };
        assert_eq!((t.icon.width, t.icon.height), (32, 32));
        assert!(t.icon.data.chunks(4).any(|p| p[3] == 255));
    }

    #[test]
    fn escape_restores_and_timeout_ends_move_mode() {
        let mut rig = Rig::new("owl", "");
        let home = rig.surface.rect();
        rig.send(Event::Command(Command::Move));
        rig.send(Event::Key(Key {
            code: KeyCode::Left,
            shift: true,
        }));
        assert_eq!(rig.surface.rect().unwrap().x, home.unwrap().x - 10);
        let shell = rig.send(Event::Key(Key {
            code: KeyCode::Escape,
            shift: false,
        }));
        assert_eq!(rig.surface.rect(), home);
        assert!(!rig.surface.interactive);
        assert!(!shell.iter().any(|e| matches!(e, Effect::SaveState(_))));

        rig.send(Event::Command(Command::Move));
        rig.now.instant += crate::movemode::TIMEOUT + Duration::from_millis(1);
        rig.send(Event::Deadline);
        assert!(!rig.surface.interactive, "timed out");
        assert!(!rig.badge.is_moving());
    }

    #[test]
    fn drag_events_are_ignored_outside_move_mode() {
        let mut rig = Rig::new("owl", "");
        let home = rig.surface.rect();
        rig.send(Event::DragStart(Point::new(1870, 40)));
        rig.send(Event::DragEnd(Point::new(10, 10)));
        assert_eq!(rig.surface.rect(), home);
    }

    #[test]
    fn tray_commands() {
        let mut rig = Rig::new("owl", "");
        rig.send(Event::Command(Command::SetSize(128)));
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1920 - 128 - 16, 16, 128, 128))
        );
        let tray = rig
            .shell
            .iter()
            .rev()
            .find_map(|e| {
                if let Effect::SetTray(t) = e {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(tray.menu[1].children[2].checked, Some(true));

        rig.send(Event::Command(Command::SetAnimation(Animation::Static)));
        assert_eq!(rig.badge.next_deadline(), None);
        assert_eq!(
            rig.send(Event::Command(Command::Reload)),
            vec![Effect::ReloadConfig]
        );
        assert_eq!(rig.send(Event::Command(Command::Quit)), vec![Effect::Exit]);
    }

    fn tray_of(shell: &[Effect]) -> Option<TrayState> {
        shell.iter().rev().find_map(|e| match e {
            Effect::SetTray(t) => Some(t.clone()),
            _ => None,
        })
    }

    #[test]
    fn checking_for_updates_shows_on_the_crate_and_in_the_menu() {
        let mut rig = Rig::new("owl", "animation = \"static\"");
        let hostname = rig.surface.frames.last().unwrap().clone();
        assert_eq!(rig.badge.next_deadline(), None);

        let shell = rig.send(Event::Command(Command::CheckUpdate));
        assert!(shell.contains(&Effect::Update(Job::Check)));
        assert_eq!(
            tray_of(&shell).unwrap().menu[6].label,
            "Checking for updates…"
        );
        let checking = rig.surface.frames.last().unwrap().clone();
        assert_ne!(checking, hostname, "CHECKING on the crate");
        assert_eq!(
            (checking.width, checking.height),
            (hostname.width, hostname.height)
        );
        // A second click while it runs starts nothing.
        assert!(rig.send(Event::Command(Command::CheckUpdate)).is_empty());
        assert!(rig.send(Event::Command(Command::InstallUpdate)).is_empty());

        // Up to date: said for a few seconds, then the hostname is back
        // and the badge sleeps again.
        let shell = rig.send(Event::Command(Command::UpdateNews(News::UpToDate)));
        assert_eq!(tray_of(&shell).unwrap().menu[6].label, "Check for updates");
        assert_ne!(rig.surface.frames.last(), Some(&checking));
        assert_eq!(
            rig.badge.next_deadline(),
            Some(rig.now.instant + Notice::UpToDate.duration())
        );
        rig.advance_to_deadline();
        assert_eq!(rig.surface.frames.last(), Some(&hostname));
        assert_eq!(rig.badge.next_deadline(), None);
        assert_eq!(
            rig.surface.places.len(),
            1,
            "a notice never moves the badge"
        );
    }

    #[test]
    fn a_found_update_is_offered_then_installed() {
        let mut rig = Rig::new("owl", "animation = \"static\"");
        // Nothing found yet: nothing to install.
        assert!(rig.send(Event::Command(Command::InstallUpdate)).is_empty());

        let v = "1.3.0".parse().unwrap();
        rig.send(Event::Command(Command::CheckUpdate));
        let shell = rig.send(Event::Command(Command::UpdateNews(News::Available(v))));
        assert_eq!(
            tray_of(&shell).unwrap().menu[6].label,
            "Install update 1.3.0"
        );
        // The offer outlives the notice.
        rig.advance_to_deadline();
        assert_eq!(rig.badge.next_deadline(), None);

        let shell = rig.send(Event::Command(Command::InstallUpdate));
        assert!(shell.contains(&Effect::Update(Job::Install)));
        assert_eq!(tray_of(&shell).unwrap().menu[6].label, "Updating…");
        // It failed (offline): said, and the offer stands.
        let shell = rig.send(Event::Command(Command::UpdateNews(News::Failed)));
        assert_eq!(
            tray_of(&shell).unwrap().menu[6].label,
            "Install update 1.3.0"
        );
        // The new badge hears it was installed.
        let shell = rig.send(Event::Command(Command::UpdateNews(News::Updated)));
        assert_eq!(tray_of(&shell).unwrap().menu[6].label, "Check for updates");
    }

    #[test]
    fn a_job_that_never_answers_fails_by_itself() {
        let mut rig = Rig::new("owl", "animation = \"static\"");
        rig.send(Event::Command(Command::CheckUpdate));
        let checking = rig.surface.frames.last().unwrap().clone();
        rig.advance_to_deadline();
        assert_ne!(rig.surface.frames.last(), Some(&checking), "FAILED");
        assert_eq!(
            rig.badge.next_deadline(),
            Some(rig.now.instant + Notice::Failed.duration())
        );
        // And another check may start.
        assert!(
            rig.send(Event::Command(Command::CheckUpdate))
                .contains(&Effect::Update(Job::Check))
        );
    }

    #[test]
    fn good_news_plays_the_signature_action() {
        let mut rig = Rig::new("owl", "interval = [600, 600]");
        let rest = rig.badge.next_deadline().unwrap();
        rig.send(Event::Command(Command::UpdateNews(News::UpToDate)));
        assert!(
            rig.badge.next_deadline().unwrap() < rest,
            "an action started now, not at the next idle time"
        );
        let frames = rig.surface.frames.len();
        for _ in 0..3 {
            rig.advance_to_deadline();
        }
        assert!(rig.surface.frames.len() > frames, "it plays");
    }

    #[test]
    fn config_reload_applies() {
        let mut rig = Rig::new("owl", "");
        let frames = rig.surface.frames.len();
        rig.send(Event::ConfigTouched(Config::parse(
            "label = \"none\"\nposition = \"bottom-left\"",
        )));
        assert_eq!(
            rig.surface.frames.len(),
            frames + 1,
            "label removed → new frame"
        );
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(16, 1040 - 64 - 16, 64, 64))
        );
    }

    #[test]
    fn the_fullscreen_policy_reaches_the_surface_once_per_change() {
        let mut rig = Rig::new("owl", "");
        assert_eq!(rig.surface.fullscreen, Some(Fullscreen::Hide), "default");
        let count =
            |rig: &Rig| rig.shell.len() + rig.surface.places.len() + rig.surface.frames.len();
        let before = count(&rig);
        let fx = rig.badge.handle(
            Event::ConfigTouched(Config::parse("fullscreen = \"show\"")),
            rig.now,
        );
        assert_eq!(fx, vec![Effect::SetFullscreen(Fullscreen::Show)]);
        apply(&mut rig.surface, fx);
        assert_eq!(rig.surface.fullscreen, Some(Fullscreen::Show));
        let fx = rig.badge.handle(
            Event::ConfigTouched(Config::parse("fullscreen = \"show\"")),
            rig.now,
        );
        assert!(fx.is_empty(), "no change, no effect: {fx:?}");
        assert_eq!(count(&rig), before);
    }

    #[test]
    fn a_new_catalog_brings_user_mascots_in_and_out() {
        let mut rig = Rig::new("owl", "mascot = \"zork\"");
        assert_eq!(rig.badge.resolved().mascot, "owl", "no zork yet");
        let mut zork = Catalog::bundled().get("otter").unwrap().clone();
        zork.id = "zork".into();
        zork.aliases.clear();
        let user = Catalog::with_user(bundle().mascots.clone(), vec![zork]);
        let frames = rig.surface.frames.len();
        let fx = rig.badge.set_catalog(Arc::new(user), rig.now);
        apply(&mut rig.surface, fx);
        assert_eq!(rig.badge.resolved().mascot, "zork");
        assert_eq!(rig.surface.frames.len(), frames + 1, "zork on screen");
        // Removed again (the folder was deleted): back to the owl, no panic.
        let fx = rig
            .badge
            .set_catalog(Arc::new(Catalog::bundled().clone()), rig.now);
        apply(&mut rig.surface, fx);
        assert_eq!(rig.badge.resolved().mascot, "owl");
        rig.advance_to_deadline();
    }

    #[test]
    fn fallback_hosts_cache_their_mascot_once() {
        let rig = Rig::new("buildbox", "");
        let saves: Vec<&State> = rig
            .shell
            .iter()
            .filter_map(|e| {
                if let Effect::SaveState(s) = e {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(saves.len(), 1);
        // Any bundled mascot may be picked; which one depends on the catalog.
        let picked = &saves[0].assignments["buildbox"];
        assert!(Catalog::bundled().get(picked).is_some());
        let cached = Rig::with_state("buildbox", "", saves[0].clone());
        assert!(
            !cached
                .shell
                .iter()
                .any(|e| matches!(e, Effect::SaveState(_)))
        );
    }

    #[test]
    fn long_labels_never_grow_the_badge() {
        let rig = Rig::new("prod-database-primary-01.lab.example", "");
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1920 - 64 - 16, 16, 64, 64))
        );
    }
}
