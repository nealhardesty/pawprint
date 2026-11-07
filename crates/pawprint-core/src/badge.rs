//! The badge as a pure state machine (SPEC §10.1): backends feed it
//! [`Event`]s and execute the [`Effect`]s it returns, then sleep until
//! [`Badge::next_deadline`] or the next native event.

use std::time::Instant;

use pawprint_sprite::{Bundle, Mascot, RgbaImage};

use crate::anim::Scheduler;
use crate::catalog::Catalog;
use crate::config::{Animation, Config, Layer};
use crate::geom::{PhysicalRect, Point};
use crate::identity::{self, Resolved};
use crate::layout;
use crate::menu::{self, TrayState};
use crate::movemode::{Key, MoveMode, Outcome};
use crate::render::{self, BadgeSpec, Frame};
use crate::state::State;
use crate::surface::{DisplayInfo, select_display};

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
    SetTray(TrayState),
    /// Write this to the state file atomically.
    SaveState(State),
    /// Re-read the config file and send `Event::ConfigTouched`.
    ReloadConfig,
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
}

pub struct Badge<'c> {
    catalog: &'c Catalog,
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
    presented: Option<RgbaImage>,
    tray: Option<TrayState>,
}

impl<'c> Badge<'c> {
    pub fn new(catalog: &'c Catalog, assets: &'c Bundle, inputs: Inputs, now: Instant) -> Self {
        let Inputs {
            hostname,
            config,
            state,
            flags,
            seed,
        } = inputs;
        let resolved = identity::resolve(
            &hostname,
            &config,
            &state,
            &flags,
            catalog,
            &assets.accents,
            false,
        );
        let anim = Scheduler::new(
            resolved.settings.animation.value,
            resolved.settings.interval.value,
            seed,
            now,
        );
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
        }
    }

    pub fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    pub fn is_moving(&self) -> bool {
        self.moving.is_some()
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
        anim.into_iter().chain(moving).min()
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
            Command::Quit => out.push(Effect::Exit),
            Command::SetSize(size) => {
                self.flags.size = Some(size);
                self.reresolve(now, out);
            }
            Command::SetAnimation(mode) => {
                self.flags.animation = Some(mode);
                self.reresolve(now, out);
            }
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
            self.catalog,
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

    fn target(&self) -> Option<&DisplayInfo> {
        select_display(&self.displays, &self.resolved.settings.display.value)
    }

    fn mascot(&self) -> &'c Mascot {
        self.catalog
            .get(&self.resolved.mascot)
            .expect("resolved mascots exist")
    }

    /// Emit Place / Present / SetTray only for what actually changed.
    fn refresh(&mut self, out: &mut Vec<Effect>) {
        let tray = TrayState {
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
            ),
        };
        if self.tray.as_ref() != Some(&tray) {
            self.tray = Some(tray.clone());
            out.push(Effect::SetTray(tray));
        }

        let Some(display) = self.target() else { return };
        let mascot = self.mascot();
        let s = &self.resolved.settings;
        let spec = BadgeSpec {
            mascot,
            frame: self.anim.frame(mascot),
            label: self.resolved.label.as_deref(),
            accent: self.resolved.accent.rgb,
            size: s.size.value,
            label_size: s.label_size.value,
            anchor: self.resolved.position.value.anchor,
            display_scale: display.scale,
        };
        let img = render::render_badge(&spec, &self.assets.crate_art, &self.assets.font);
        let rect = match &self.moving {
            Some(m) => PhysicalRect {
                width: img.width,
                height: img.height,
                ..m.rect()
            },
            None => layout::place(
                self.resolved.position.value,
                img.width,
                img.height,
                display.work_area,
                display.scale,
            ),
        };
        if self.placed != Some(rect) {
            self.placed = Some(rect);
            out.push(Effect::Place(rect));
        }
        if self.presented.as_ref() != Some(&img) {
            out.push(Effect::Present(Frame::from_rgba(&img)));
            self.presented = Some(img);
        }
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
            };
            let mut badge = Badge::new(Catalog::bundled(), bundle(), inputs, now.instant);
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
        assert_eq!(
            saves[0].assignments.get("buildbox").map(String::as_str),
            Some("owl")
        );
        let cached = Rig::with_state("buildbox", "", saves[0].clone());
        assert!(
            !cached
                .shell
                .iter()
                .any(|e| matches!(e, Effect::SaveState(_)))
        );
    }

    #[test]
    fn long_labels_grow_the_badge_inward() {
        let rig = Rig::new("prod-owl-02.lab.example", "");
        assert_eq!(
            rig.surface.rect(),
            Some(PhysicalRect::new(1920 - 142 - 16, 16, 142, 90))
        );
    }
}
