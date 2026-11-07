//! `pawprint-sprite` compiles PAWPRINT's text pixel-art sources (SPEC §12)
//! into indexed sprites. The same crate runs in `pawprint-core`'s `build.rs`,
//! at runtime for user mascots, in `pawdev`, and in `pawprint mascot check`,
//! so they can never disagree about what a valid mascot is.

mod bundle;
mod color;
mod compile;
mod grid;
mod image;
mod library;
mod parts;
mod source;

pub use bundle::{Bundle, BundleReport, build_bundle, check_gates};
pub use color::{Ramp, Rgb, ramp};
pub use compile::{Action, Compiled, FrameRef, Mascot, compile, compile_dir};
pub use image::RgbaImage;
pub use library::{Library, Prop};
pub use parts::{CrateArt, CrateColor, Font, IndexedLayer, Shade};
pub use source::When;

use std::fmt;

/// Mascot frames are always 32 × 32 art pixels (SPEC §4.2).
pub const FRAME_SIZE: u32 = 32;

/// Where a mascot comes from. Bundled art is held to stricter rules
/// (master palette names only, warnings fail the build, signature action
/// required) than user art (SPEC §12.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Bundled,
    User,
}

/// Valid mascot categories (SPEC §12.3).
pub const CATEGORIES: [&str; 6] = [
    "mammals",
    "cats-dogs",
    "birds",
    "aquatic",
    "critters",
    "oddballs",
];

/// One validation finding, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    pub context: String,
    pub message: String,
}

impl Diag {
    pub fn new(context: impl Into<String>, message: impl Into<String>) -> Self {
        Diag {
            context: context.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.context.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.context, self.message)
        }
    }
}

/// Errors and warnings collected while compiling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub errors: Vec<Diag>,
    pub warnings: Vec<Diag>,
}

impl Report {
    pub fn error(&mut self, context: impl Into<String>, message: impl Into<String>) {
        self.errors.push(Diag::new(context, message));
    }

    pub fn warn(&mut self, context: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Diag::new(context, message));
    }

    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty()
    }

    /// Append another report, prefixing each finding's context.
    pub fn absorb(&mut self, prefix: &str, other: Report) {
        let join = |d: Diag| {
            let context = if d.context.is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}: {}", d.context)
            };
            Diag {
                context,
                message: d.message,
            }
        };
        self.errors.extend(other.errors.into_iter().map(join));
        self.warnings.extend(other.warnings.into_iter().map(join));
    }
}

/// Mascot ID grammar: lowercase ASCII kebab-case, at most 32 characters
/// (SPEC §2).
pub fn is_valid_id(id: &str) -> bool {
    id.len() <= 32 && is_kebab(id)
}

/// `[a-z0-9]+(-[a-z0-9]+)*`
pub(crate) fn is_kebab(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Lowercase, then drop everything that is not `[a-z0-9]` (SPEC §5.3).
/// Aliases are compared in this form, so `polar-bear`, `polar_bear`, and
/// `polarbear` are the same alias.
pub fn squash(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_grammar() {
        let cases = [
            ("owl", true),
            ("cat-maine-coon-black", true),
            ("r2d2", true),
            ("", false),
            ("Owl", false),
            ("owl-", false),
            ("-owl", false),
            ("owl--cat", false),
            ("owl_cat", false),
            ("abcdefghijklmnopqrstuvwxyz-01234", true),
            ("abcdefghijklmnopqrstuvwxyz-012345", false),
        ];
        for (id, ok) in cases {
            assert_eq!(is_valid_id(id), ok, "{id:?}");
        }
    }

    #[test]
    fn squash_normalizes() {
        let cases = [
            ("polar-bear", "polarbear"),
            ("Polar_Bear", "polarbear"),
            ("red panda 01", "redpanda01"),
            ("é-x", "x"),
        ];
        for (input, want) in cases {
            assert_eq!(squash(input), want);
        }
    }
}
