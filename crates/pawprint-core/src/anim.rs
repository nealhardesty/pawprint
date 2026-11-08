//! The idle-animation scheduler (SPEC §7.1). It never ticks: it only knows
//! its next deadline, and advances when the shell wakes it at that time.

use std::time::{Duration, Instant};

use pawprint_sprite::{Action, Mascot, When};

use crate::config::Animation;
use crate::hash::Rng;

/// At night, `when = "night"` actions of nocturnal mascots weigh this much
/// more, so they are livelier after dark (SPEC §12.5).
pub const NOCTURNAL_NIGHT_BOOST: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Off,
    Resting {
        until: Instant,
    },
    Playing {
        action: usize,
        step: usize,
        until: Instant,
    },
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    mode: Animation,
    interval: [u32; 2],
    rng: Rng,
    phase: Phase,
    /// Preview mode: play every action in turn, ignoring weights and
    /// `when`; this is the next one to try.
    cycle: Option<usize>,
}

impl Scheduler {
    pub fn new(mode: Animation, interval: [u32; 2], seed: u64, now: Instant) -> Scheduler {
        let mut s = Scheduler {
            mode,
            interval,
            rng: Rng::new(seed),
            phase: Phase::Off,
            cycle: None,
        };
        s.restart(now);
        s
    }

    /// Preview mode (`pawprint preview`): every action in file order, one
    /// after another, with the configured rest between them.
    pub fn cycle_all(&mut self, now: Instant) {
        self.cycle = Some(0);
        self.restart(now);
    }

    /// Return to rest and schedule the next action (or nothing, in static mode).
    pub fn restart(&mut self, now: Instant) {
        self.phase = match self.mode {
            Animation::Static => Phase::Off,
            Animation::Idle => Phase::Resting {
                until: now + self.rest_duration(),
            },
        };
    }

    /// Change mode or interval; restarts only if something changed.
    pub fn configure(&mut self, mode: Animation, interval: [u32; 2], now: Instant) {
        if (mode, interval) != (self.mode, self.interval) {
            self.mode = mode;
            self.interval = interval;
            self.restart(now);
        }
    }

    fn rest_duration(&mut self) -> Duration {
        let [lo, hi] = self.interval;
        Duration::from_millis(self.rng.range(u64::from(lo) * 1000, u64::from(hi) * 1000))
    }

    /// When the shell must wake us next. `None` means never (static mode).
    pub fn next_deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::Off => None,
            Phase::Resting { until } | Phase::Playing { until, .. } => Some(until),
        }
    }

    /// Index into `mascot.frames` to show now.
    pub fn frame(&self, mascot: &Mascot) -> usize {
        match self.phase {
            Phase::Playing { action, step, .. } => {
                usize::from(mascot.actions[action].frames[step].frame)
            }
            _ => 0,
        }
    }

    /// The action playing now, if any.
    pub fn playing<'m>(&self, mascot: &'m Mascot) -> Option<&'m Action> {
        match self.phase {
            Phase::Playing { action, .. } => mascot.actions.get(action),
            _ => None,
        }
    }

    /// Advance if the deadline has passed. Times are measured from `now`, so
    /// a late wakeup (sleep, suspend) never causes a burst of catch-up frames.
    pub fn on_deadline(&mut self, now: Instant, local_hour: u8, mascot: &Mascot) {
        match self.phase {
            Phase::Off => {}
            Phase::Resting { until } | Phase::Playing { until, .. } if now < until => {}
            Phase::Resting { .. } => {
                let pick = match self.cycle.as_mut() {
                    Some(next) => {
                        let n = mascot.actions.len();
                        let found = (0..n)
                            .map(|i| (*next + i) % n)
                            .find(|&i| !mascot.actions[i].frames.is_empty());
                        *next = found.map_or(0, |i| i + 1);
                        found
                    }
                    None => pick_action(mascot, local_hour, self.rng.next_u64()),
                };
                self.phase = match pick {
                    Some(action) => {
                        let ms = mascot.actions[action].frames[0].ms;
                        Phase::Playing {
                            action,
                            step: 0,
                            until: now + Duration::from_millis(u64::from(ms)),
                        }
                    }
                    None => Phase::Resting {
                        until: now + self.rest_duration(),
                    },
                };
            }
            Phase::Playing { action, step, .. } => {
                let frames = &mascot.actions[action].frames;
                self.phase = match frames.get(step + 1) {
                    Some(next) => Phase::Playing {
                        action,
                        step: step + 1,
                        until: now + Duration::from_millis(u64::from(next.ms)),
                    },
                    None => Phase::Resting {
                        until: now + self.rest_duration(),
                    },
                };
            }
        }
    }
}

/// Weighted choice among the actions whose `when` matches the local hour.
pub fn pick_action(mascot: &Mascot, local_hour: u8, roll: u64) -> Option<usize> {
    let night = When::Night.matches(local_hour);
    let weight = |a: &Action| -> u64 {
        if !a.when.matches(local_hour) || a.frames.is_empty() {
            0
        } else if mascot.nocturnal && night && a.when == When::Night {
            u64::from(a.weight * NOCTURNAL_NIGHT_BOOST)
        } else {
            u64::from(a.weight)
        }
    };
    let total: u64 = mascot.actions.iter().map(weight).sum();
    if total == 0 {
        return None;
    }
    let mut r = roll % total;
    for (i, a) in mascot.actions.iter().enumerate() {
        let w = weight(a);
        if r < w {
            return Some(i);
        }
        r -= w;
    }
    unreachable!("roll is below the total weight")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pawprint_sprite::FrameRef;

    /// (name, weight, when, [(frame, ms)])
    type ActionSpec<'a> = (&'a str, u32, When, &'a [(u16, u16)]);

    pub fn mascot(actions: &[ActionSpec<'_>]) -> Mascot {
        Mascot {
            id: "test".into(),
            name: "Test".into(),
            category: "birds".into(),
            aliases: vec![],
            accent: "violet".into(),
            fallback: true,
            nocturnal: false,
            base: None,
            colors: vec![],
            frames: vec![vec![0; 1024]; 4],
            actions: actions
                .iter()
                .map(|(name, weight, when, frames)| Action {
                    name: name.to_string(),
                    weight: *weight,
                    when: *when,
                    template: None,
                    frames: frames
                        .iter()
                        .map(|&(frame, ms)| FrameRef { frame, ms })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn static_mode_never_wakes() {
        let s = Scheduler::new(Animation::Static, [8, 20], 1, Instant::now());
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn rests_plays_every_frame_then_rests() {
        let m = mascot(&[("blink", 1, When::Any, &[(1, 140), (2, 200)])]);
        let t0 = Instant::now();
        let mut s = Scheduler::new(Animation::Idle, [8, 20], 7, t0);
        let rest_until = s.next_deadline().unwrap();
        let wait = rest_until - t0;
        assert!(
            wait >= Duration::from_secs(8) && wait <= Duration::from_secs(20),
            "{wait:?}"
        );
        assert_eq!(s.frame(&m), 0);

        s.on_deadline(rest_until - Duration::from_millis(1), 12, &m);
        assert_eq!(
            s.next_deadline(),
            Some(rest_until),
            "an early wakeup changes nothing"
        );

        s.on_deadline(rest_until, 12, &m);
        assert_eq!(
            (s.frame(&m), s.next_deadline()),
            (1, Some(rest_until + Duration::from_millis(140)))
        );
        assert_eq!(s.playing(&m).unwrap().name, "blink");

        let t = s.next_deadline().unwrap();
        s.on_deadline(t, 12, &m);
        assert_eq!(s.frame(&m), 2);

        let t = s.next_deadline().unwrap();
        s.on_deadline(t, 12, &m);
        assert_eq!(s.frame(&m), 0, "back to rest");
        assert!(s.next_deadline().unwrap() >= t + Duration::from_secs(8));
    }

    #[test]
    fn no_eligible_action_keeps_resting() {
        let m = mascot(&[("doze", 1, When::Night, &[(1, 200)])]);
        let t0 = Instant::now();
        let mut s = Scheduler::new(Animation::Idle, [8, 8], 7, t0);
        let t = s.next_deadline().unwrap();
        s.on_deadline(t, 12, &m);
        assert_eq!(s.frame(&m), 0);
        assert_eq!(s.next_deadline(), Some(t + Duration::from_secs(8)));
        s.on_deadline(t + Duration::from_secs(8), 23, &m);
        assert_eq!(s.frame(&m), 1, "plays at night");
    }

    #[test]
    fn weights_and_when() {
        let mut m = mascot(&[
            ("a", 6, When::Any, &[(1, 200)]),
            ("b", 3, When::Day, &[(1, 200)]),
            ("c", 1, When::Night, &[(1, 200)]),
        ]);
        // `rolls` is a multiple of the total weight, so counts are exact.
        let count = |m: &Mascot, hour, rolls: u64| {
            let mut n = [0u32; 3];
            for roll in 0..rolls {
                n[pick_action(m, hour, roll).unwrap()] += 1;
            }
            n
        };
        assert_eq!(
            count(&m, 12, 900),
            [600, 300, 0],
            "by day: 6:3, night action excluded"
        );
        assert_eq!(count(&m, 22, 700), [600, 0, 100], "by night: 6:1");
        m.nocturnal = true;
        assert_eq!(
            count(&m, 22, 900),
            [600, 0, 300],
            "nocturnal: night actions weigh 3x"
        );
        assert_eq!(pick_action(&mascot(&[]), 12, 5), None);
    }

    #[test]
    fn preview_cycles_every_action_in_order() {
        let m = mascot(&[
            ("a", 6, When::Any, &[(1, 100)]),
            ("empty", 1, When::Any, &[]),
            ("b", 1, When::Night, &[(2, 100)]),
        ]);
        let t0 = Instant::now();
        let mut s = Scheduler::new(Animation::Idle, [1, 1], 3, t0);
        s.cycle_all(t0);
        let mut played = Vec::new();
        for _ in 0..8 {
            let t = s.next_deadline().unwrap();
            s.on_deadline(t, 12, &m);
            if let Some(a) = s.playing(&m) {
                played.push(a.name.clone());
            }
        }
        assert_eq!(played, ["a", "b", "a", "b"], "night actions too, by day");
    }

    #[test]
    fn configure_restarts_only_on_change() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(Animation::Idle, [8, 20], 1, t0);
        let d = s.next_deadline();
        s.configure(Animation::Idle, [8, 20], t0 + Duration::from_secs(1));
        assert_eq!(s.next_deadline(), d);
        s.configure(Animation::Static, [8, 20], t0);
        assert_eq!(s.next_deadline(), None);
    }
}
