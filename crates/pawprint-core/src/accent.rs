//! Accent colors (SPEC §5.4, Appendix B).

use std::fmt;

use pawprint_sprite::Rgb;

use crate::hash::{fnv1a64, splitmix64};

/// The accent palette's frozen fallback range: always the first sixteen.
pub const FALLBACK_ACCENTS: usize = 16;

/// Index of a fallback host's accent: `splitmix64(fnv1a64(short)) mod 16`.
pub fn fallback_index(short: &str) -> usize {
    (splitmix64(fnv1a64(short.as_bytes())) % FALLBACK_ACCENTS as u64) as usize
}

/// Look up an accent name or parse `#rrggbb`.
pub fn lookup(name: &str, accents: &[(String, Rgb)]) -> Option<Rgb> {
    accents
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, c)| *c)
        .or_else(|| Rgb::parse_hex(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentSource {
    Config,
    Mascot,
    Fallback,
}

impl fmt::Display for AccentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AccentSource::Config => "from config",
            AccentSource::Mascot => "from mascot",
            AccentSource::Fallback => "hashed from hostname",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accent {
    /// Palette name or `#rrggbb`.
    pub name: String,
    pub rgb: Rgb,
    pub source: AccentSource,
}

/// Pick the accent (SPEC §5.4). `fallback_mascot` is true when the mascot
/// came from rule 2 or 5. Returns a warning when the configured accent is
/// unknown (and then behaves as `auto`).
pub fn resolve(
    setting: &str,
    fallback_mascot: bool,
    mascot_accent: &str,
    short: &str,
    accents: &[(String, Rgb)],
) -> (Accent, Option<String>) {
    let mut warning = None;
    if setting != "auto" {
        match lookup(setting, accents) {
            Some(rgb) => {
                return (
                    Accent {
                        name: setting.to_string(),
                        rgb,
                        source: AccentSource::Config,
                    },
                    None,
                );
            }
            None => {
                warning = Some(format!(
                    "accent {setting:?} is not a palette name or #rrggbb (see `pawprint list --accents`); using auto"
                ))
            }
        }
    }
    if !fallback_mascot && let Some(rgb) = lookup(mascot_accent, accents) {
        return (
            Accent {
                name: mascot_accent.to_string(),
                rgb,
                source: AccentSource::Mascot,
            },
            warning,
        );
    }
    let (name, rgb) = &accents[fallback_index(short) % accents.len().min(FALLBACK_ACCENTS)];
    (
        Accent {
            name: name.clone(),
            rgb: *rgb,
            source: AccentSource::Fallback,
        },
        warning,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::bundle;

    /// SPEC Appendix B: order and values are frozen.
    #[test]
    fn palette_is_frozen() {
        let want = [
            ("violet", "#7b5ea7"),
            ("pine", "#2f6b4f"),
            ("ochre", "#c8962e"),
            ("copper", "#b8673a"),
            ("slate", "#4f6d8f"),
            ("cranberry", "#9e2b45"),
            ("ice", "#8fc3d9"),
            ("marigold", "#e8912d"),
            ("plum", "#6e3b62"),
            ("honey", "#d9a441"),
            ("teal", "#2a8c87"),
            ("brick", "#a4462f"),
            ("moss", "#6f7f3a"),
            ("rose", "#d77a91"),
            ("sky", "#5b9bd5"),
            ("charcoal", "#4a4a4f"),
        ];
        let got: Vec<(&str, String)> = bundle()
            .accents
            .iter()
            .take(16)
            .map(|(n, c)| (n.as_str(), c.to_hex()))
            .collect();
        let want: Vec<(&str, String)> = want.iter().map(|(n, h)| (*n, h.to_string())).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn fallback_index_is_frozen() {
        // Computed independently of this code; these must never change.
        let cases = [("buildbox", 5), ("prod-db-01", 14), ("owl", 14), ("", 0)];
        for (short, want) in cases {
            assert_eq!(fallback_index(short), want, "{short:?}");
        }
        assert_eq!(
            bundle().accents[5].0,
            "cranberry",
            "buildbox gets a cranberry crate"
        );
        let spread: std::collections::BTreeSet<usize> = (0..200)
            .map(|i| fallback_index(&format!("host{i}")))
            .collect();
        assert_eq!(spread.len(), 16, "all sixteen accents get used");
    }

    #[test]
    fn resolution_order() {
        let accents = &bundle().accents;
        let (a, w) = resolve("teal", false, "violet", "owl", accents);
        assert_eq!(
            (a.name.as_str(), a.source, w),
            ("teal", AccentSource::Config, None)
        );
        let (a, _) = resolve("#010203", true, "violet", "owl", accents);
        assert_eq!((a.rgb, a.source), (Rgb(1, 2, 3), AccentSource::Config));
        let (a, _) = resolve("auto", false, "violet", "owl", accents);
        assert_eq!(
            (a.name.as_str(), a.source),
            ("violet", AccentSource::Mascot)
        );
        let (a, _) = resolve("auto", true, "violet", "buildbox", accents);
        assert_eq!(a.source, AccentSource::Fallback);
        assert_eq!(a.name, accents[fallback_index("buildbox")].0);
        let (a, w) = resolve("mauve", false, "violet", "owl", accents);
        assert_eq!(a.source, AccentSource::Mascot);
        assert!(w.unwrap().contains("mauve"));
    }
}
