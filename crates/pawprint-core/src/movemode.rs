//! Move mode (SPEC §6.3): drag math, constraint to the usable area,
//! nudging, Escape, and the 30-second timeout. Backends report raw pointer
//! and key events; every decision is made here.

use std::time::{Duration, Instant};

use crate::geom::{PhysicalRect, Point};

/// Move mode ends after this long without interaction.
pub const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub shift: bool,
}

/// What the badge should do after a move-mode event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Still in move mode; show the badge here.
    Moving(PhysicalRect),
    /// Move mode is over. `save` is false when cancelled, and `rect` is then
    /// the original position.
    Done { rect: PhysicalRect, save: bool },
}

#[derive(Debug, Clone)]
pub struct MoveMode {
    original: PhysicalRect,
    rect: PhysicalRect,
    /// Pointer and rect position when the drag started.
    drag: Option<(Point, Point)>,
    last: Instant,
}

impl MoveMode {
    pub fn new(rect: PhysicalRect, now: Instant) -> MoveMode {
        MoveMode {
            original: rect,
            rect,
            drag: None,
            last: now,
        }
    }

    pub fn rect(&self) -> PhysicalRect {
        self.rect
    }

    pub fn deadline(&self) -> Instant {
        self.last + TIMEOUT
    }

    pub fn drag_start(&mut self, pointer: Point, now: Instant) {
        self.drag = Some((pointer, Point::new(self.rect.x, self.rect.y)));
        self.last = now;
    }

    pub fn drag_move(&mut self, pointer: Point, usable: PhysicalRect, now: Instant) -> Outcome {
        if let Some((p0, r0)) = self.drag {
            let moved = PhysicalRect {
                x: r0.x + pointer.x - p0.x,
                y: r0.y + pointer.y - p0.y,
                ..self.rect
            };
            self.rect = moved.clamped_into(usable);
        }
        self.last = now;
        Outcome::Moving(self.rect)
    }

    /// Releasing the pointer saves and ends move mode.
    pub fn drag_end(&mut self, pointer: Point, usable: PhysicalRect, now: Instant) -> Outcome {
        if self.drag.is_none() {
            self.last = now;
            return Outcome::Moving(self.rect);
        }
        self.drag_move(pointer, usable, now);
        self.drag = None;
        Outcome::Done {
            rect: self.rect,
            save: true,
        }
    }

    /// Arrows nudge by 1 logical pixel (10 with Shift), Enter saves, Escape
    /// cancels.
    pub fn key(&mut self, key: Key, usable: PhysicalRect, scale: f64, now: Instant) -> Outcome {
        self.last = now;
        let step = ((if key.shift { 10.0 } else { 1.0 }) * scale)
            .round()
            .max(1.0) as i32;
        let (dx, dy) = match key.code {
            KeyCode::Left => (-step, 0),
            KeyCode::Right => (step, 0),
            KeyCode::Up => (0, -step),
            KeyCode::Down => (0, step),
            KeyCode::Enter => {
                return Outcome::Done {
                    rect: self.rect,
                    save: true,
                };
            }
            KeyCode::Escape => {
                return Outcome::Done {
                    rect: self.original,
                    save: false,
                };
            }
        };
        self.rect = PhysicalRect {
            x: self.rect.x + dx,
            y: self.rect.y + dy,
            ..self.rect
        }
        .clamped_into(usable);
        Outcome::Moving(self.rect)
    }

    /// Past the deadline, move mode ends, saving any completed move.
    pub fn timeout(&mut self, now: Instant) -> Option<Outcome> {
        if now < self.deadline() {
            return None;
        }
        if self.drag.is_some() {
            // A drag in progress is not complete: treat it as cancelled.
            return Some(Outcome::Done {
                rect: self.original,
                save: false,
            });
        }
        let moved = self.rect != self.original;
        Some(Outcome::Done {
            rect: if moved { self.rect } else { self.original },
            save: moved,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USABLE: PhysicalRect = PhysicalRect::new(0, 0, 1920, 1040);
    const START: PhysicalRect = PhysicalRect::new(1840, 16, 64, 64);

    fn key(code: KeyCode, shift: bool) -> Key {
        Key { code, shift }
    }

    #[test]
    fn drag_follows_the_pointer_and_release_saves() {
        let t = Instant::now();
        let mut m = MoveMode::new(START, t);
        m.drag_start(Point::new(1870, 40), t);
        assert_eq!(
            m.drag_move(Point::new(1000, 500), USABLE, t),
            Outcome::Moving(PhysicalRect::new(970, 476, 64, 64))
        );
        assert_eq!(
            m.drag_end(Point::new(1010, 500), USABLE, t),
            Outcome::Done {
                rect: PhysicalRect::new(980, 476, 64, 64),
                save: true
            }
        );
    }

    #[test]
    fn drag_is_constrained_to_the_usable_area() {
        let t = Instant::now();
        let mut m = MoveMode::new(START, t);
        m.drag_start(Point::new(1870, 40), t);
        assert_eq!(
            m.drag_move(Point::new(5000, -500), USABLE, t),
            Outcome::Moving(PhysicalRect::new(1856, 0, 64, 64))
        );
    }

    #[test]
    fn motion_without_a_press_does_nothing() {
        let t = Instant::now();
        let mut m = MoveMode::new(START, t);
        assert_eq!(
            m.drag_move(Point::new(5, 5), USABLE, t),
            Outcome::Moving(START)
        );
        assert_eq!(
            m.drag_end(Point::new(5, 5), USABLE, t),
            Outcome::Moving(START)
        );
    }

    #[test]
    fn keys_nudge_save_and_cancel() {
        let t = Instant::now();
        let mut m = MoveMode::new(START, t);
        assert_eq!(
            m.key(key(KeyCode::Left, false), USABLE, 1.0, t),
            Outcome::Moving(PhysicalRect::new(1839, 16, 64, 64))
        );
        assert_eq!(
            m.key(key(KeyCode::Down, true), USABLE, 2.0, t),
            Outcome::Moving(PhysicalRect::new(1839, 36, 64, 64))
        );
        assert_eq!(
            m.key(key(KeyCode::Up, true), USABLE, 1.0, t),
            Outcome::Moving(PhysicalRect::new(1839, 26, 64, 64))
        );
        assert_eq!(
            m.key(key(KeyCode::Enter, false), USABLE, 1.0, t),
            Outcome::Done {
                rect: PhysicalRect::new(1839, 26, 64, 64),
                save: true
            }
        );

        let mut m = MoveMode::new(START, t);
        m.key(key(KeyCode::Right, true), USABLE, 1.0, t);
        assert_eq!(
            m.key(key(KeyCode::Escape, false), USABLE, 1.0, t),
            Outcome::Done {
                rect: START,
                save: false
            }
        );
    }

    #[test]
    fn timeout_after_30s_without_interaction() {
        let t = Instant::now();
        let mut m = MoveMode::new(START, t);
        assert_eq!(m.deadline(), t + TIMEOUT);
        assert_eq!(m.timeout(t + Duration::from_secs(29)), None);
        assert_eq!(
            m.timeout(t + TIMEOUT),
            Some(Outcome::Done {
                rect: START,
                save: false
            }),
            "nothing moved"
        );

        let mut m = MoveMode::new(START, t);
        m.key(
            key(KeyCode::Left, false),
            USABLE,
            1.0,
            t + Duration::from_secs(10),
        );
        assert_eq!(
            m.timeout(t + TIMEOUT),
            None,
            "interaction extends the deadline"
        );
        let done = m.timeout(t + Duration::from_secs(40)).unwrap();
        assert_eq!(
            done,
            Outcome::Done {
                rect: PhysicalRect::new(1839, 16, 64, 64),
                save: true
            },
            "completed nudges are saved"
        );

        let mut m = MoveMode::new(START, t);
        m.drag_start(Point::new(0, 0), t);
        m.drag_move(Point::new(-100, 0), USABLE, t);
        assert_eq!(
            m.timeout(t + TIMEOUT),
            Some(Outcome::Done {
                rect: START,
                save: false
            }),
            "an unfinished drag is dropped"
        );
    }
}
