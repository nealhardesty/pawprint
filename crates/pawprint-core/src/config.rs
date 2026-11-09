//! `config.toml` parsing and precedence (SPEC §8.2–8.3).
//!
//! A config file never prevents the badge from starting: syntax errors,
//! unknown keys, and invalid values become warnings, and the affected keys
//! fall back to the next layer down (a glob table, the top level, then the
//! default). That includes a `mascot` or `accent` that is well formed but
//! not in the catalog ([`Config::effective_checked`]).

use std::fmt;

use crate::host::LabelCase;
use crate::layout::{Anchor, LabelSize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Animation {
    #[default]
    Idle,
    Static,
}

impl Animation {
    pub fn parse(s: &str) -> Option<Animation> {
        match s {
            "idle" => Some(Animation::Idle),
            "static" => Some(Animation::Static),
            _ => None,
        }
    }
}

impl fmt::Display for Animation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Animation::Idle => "idle",
            Animation::Static => "static",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Auto,
    Windows,
    Macos,
    X11,
    Wayland,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Backend> {
        match s {
            "auto" => Some(Backend::Auto),
            "windows" => Some(Backend::Windows),
            "macos" => Some(Backend::Macos),
            "x11" => Some(Backend::X11),
            "wayland" => Some(Backend::Wayland),
            _ => None,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Backend::Auto => "auto",
            Backend::Windows => "windows",
            Backend::Macos => "macos",
            Backend::X11 => "x11",
            Backend::Wayland => "wayland",
        })
    }
}

/// What the badge does when a fullscreen window covers its display
/// (a remote-desktop client, a video, a game): step aside, or stay on top.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Fullscreen {
    #[default]
    Hide,
    Show,
}

impl Fullscreen {
    pub fn parse(s: &str) -> Option<Fullscreen> {
        match s {
            "hide" => Some(Fullscreen::Hide),
            "show" => Some(Fullscreen::Show),
            _ => None,
        }
    }
}

impl fmt::Display for Fullscreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Fullscreen::Hide => "hide",
            Fullscreen::Show => "show",
        })
    }
}

/// The largest `offset` component, either sign, in logical pixels.
pub const MAX_OFFSET: i64 = 100_000;

/// One layer of settings: the top level, a `[hosts]` table, or flags.
/// `None` means "not set here".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub mascot: Option<String>,
    pub label: Option<String>,
    pub label_case: Option<LabelCase>,
    pub label_size: Option<LabelSize>,
    pub size: Option<u32>,
    pub position: Option<Anchor>,
    pub offset: Option<[i32; 2]>,
    pub display: Option<String>,
    pub animation: Option<Animation>,
    pub interval: Option<[u32; 2]>,
    pub accent: Option<String>,
    pub tray: Option<bool>,
    pub fullscreen: Option<Fullscreen>,
    pub backend: Option<Backend>,
}

/// A parsed config file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub top: Layer,
    /// `[hosts."<pattern>"]` tables in file order, patterns lowercased.
    pub hosts: Vec<(String, Layer)>,
    /// The same tables' keys as written in the file (same order and
    /// length as `hosts`), for editing them.
    pub host_keys: Vec<String>,
    pub warnings: Vec<String>,
}

impl Config {
    pub fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                cfg.warnings.push(format!(
                    "config.toml is not valid TOML, using defaults: {e}"
                ));
                return cfg;
            }
        };
        for (key, value) in &table {
            if key == "hosts" {
                let Some(hosts) = value.as_table() else {
                    cfg.warnings
                        .push("`hosts` must be a table of host tables".into());
                    continue;
                };
                for (pattern, host) in hosts {
                    let Some(host) = host.as_table() else {
                        cfg.warnings
                            .push(format!("[hosts.{pattern:?}] must be a table"));
                        continue;
                    };
                    let mut layer = Layer::default();
                    for (k, v) in host {
                        set_key(
                            &mut layer,
                            k,
                            v,
                            &format!("[hosts.{pattern:?}] "),
                            &mut cfg.warnings,
                        );
                    }
                    cfg.hosts.push((pattern.to_lowercase(), layer));
                    cfg.host_keys.push(pattern.clone());
                }
            } else {
                set_key(&mut cfg.top, key, value, "", &mut cfg.warnings);
            }
        }
        cfg
    }

    /// Effective settings for host `short`, with where each value came from.
    /// Only checks values on their own; see [`Config::effective_checked`].
    pub fn effective(&self, short: &str, flags: &Layer) -> Settings {
        self.effective_checked(short, flags, |_| true, |_| true).0
    }

    /// [`Config::effective`], also skipping any `mascot` that `mascot_ok`
    /// rejects and any `accent` that `accent_ok` rejects (not in the
    /// catalog), so the next layer down applies. Returns a warning for each
    /// skipped value.
    pub fn effective_checked(
        &self,
        short: &str,
        flags: &Layer,
        mascot_ok: impl Fn(&str) -> bool,
        accent_ok: impl Fn(&str) -> bool,
    ) -> (Settings, Vec<String>) {
        let short = short.to_lowercase();
        let mut globs: Vec<(usize, &String, &Layer)> = self
            .hosts
            .iter()
            .enumerate()
            .filter(|(_, (p, _))| {
                let p = host_pattern(p);
                is_glob(p) && glob_match(p, &short)
            })
            .map(|(i, (p, l))| (i, p, l))
            .collect();
        // Most specific first: longest pattern, then fewest wildcards, then file order.
        globs.sort_by_key(|(i, p, _)| {
            let p = host_pattern(p);
            (
                std::cmp::Reverse(p.len()),
                p.chars().filter(|c| matches!(c, '*' | '?')).count(),
                *i,
            )
        });

        let mut layers: Vec<(&Layer, Source)> = vec![(flags, Source::Flag)];
        if let Some((p, l)) = self
            .hosts
            .iter()
            .rev()
            .find(|(p, _)| is_exact_host(p, &short))
        {
            layers.push((l, Source::Host(p.clone())));
        }
        layers.extend(
            globs
                .into_iter()
                .map(|(_, p, l)| (l, Source::Host(p.clone()))),
        );
        layers.push((&self.top, Source::TopLevel));

        let mascot_ok = |m: &String| m == "auto" || mascot_ok(m);
        let accent_ok = |a: &String| a == "auto" || accent_ok(a);
        let mut warnings = Vec::new();
        for (l, source) in &layers {
            if let Some(m) = l.mascot.as_ref().filter(|m| !mascot_ok(m)) {
                warnings.push(format!(
                    "mascot = {m:?} ({source}) is not in the catalog (see `pawprint list`); using the next layer down"
                ));
            }
            if let Some(a) = l.accent.as_ref().filter(|a| !accent_ok(a)) {
                warnings.push(format!(
                    "accent = {a:?} ({source}) is not a palette name or #rrggbb (see `pawprint list --accents`); using the next layer down"
                ));
            }
        }

        fn pick<T: Clone>(
            layers: &[(&Layer, Source)],
            get: impl Fn(&Layer) -> Option<T>,
            default: T,
        ) -> Sourced<T> {
            layers
                .iter()
                .find_map(|(l, s)| {
                    get(l).map(|value| Sourced {
                        value,
                        source: s.clone(),
                    })
                })
                .unwrap_or(Sourced {
                    value: default,
                    source: Source::Default,
                })
        }
        let settings = Settings {
            mascot: pick(
                &layers,
                |l| l.mascot.clone().filter(mascot_ok),
                "auto".to_string(),
            ),
            label: pick(&layers, |l| l.label.clone(), "auto".to_string()),
            label_case: pick(&layers, |l| l.label_case, LabelCase::Upper),
            label_size: pick(&layers, |l| l.label_size, LabelSize::Auto),
            size: pick(&layers, |l| l.size, 64),
            position: pick(&layers, |l| l.position, Anchor::TopRight),
            offset: pick(&layers, |l| l.offset, [16, 16]),
            display: pick(&layers, |l| l.display.clone(), "primary".to_string()),
            animation: pick(&layers, |l| l.animation, Animation::Idle),
            interval: pick(&layers, |l| l.interval, [8, 20]),
            accent: pick(
                &layers,
                |l| l.accent.clone().filter(accent_ok),
                "auto".to_string(),
            ),
            tray: pick(&layers, |l| l.tray, true),
            fullscreen: pick(&layers, |l| l.fullscreen, Fullscreen::Hide),
            backend: pick(&layers, |l| l.backend, Backend::Auto),
        };
        (settings, warnings)
    }
}

impl Layer {
    /// The value of `key` set in this layer, as TOML (`"top-right"`, `64`,
    /// `[8, 20]`), or `None` if it isn't set here or isn't a key.
    pub fn get(&self, key: &str) -> Option<toml::Value> {
        let s = |v: &dyn fmt::Display| toml::Value::from(v.to_string());
        let pair = |[a, b]: [i64; 2]| toml::Value::Array(vec![a.into(), b.into()]);
        Some(match key {
            "mascot" => s(self.mascot.as_ref()?),
            "label" => s(self.label.as_ref()?),
            "label_case" => s(&self.label_case?),
            "label_size" => s(&self.label_size?),
            "size" => i64::from(self.size?).into(),
            "position" => s(&self.position?),
            "offset" => pair(self.offset?.map(i64::from)),
            "display" => s(self.display.as_ref()?),
            "animation" => s(&self.animation?),
            "interval" => pair(self.interval?.map(i64::from)),
            "accent" => s(self.accent.as_ref()?),
            "tray" => self.tray?.into(),
            "fullscreen" => s(&self.fullscreen?),
            "backend" => s(&self.backend?),
            _ => return None,
        })
    }
}

impl Settings {
    /// The effective value of `key`, as TOML, and where it came from.
    /// `None` for an unknown key.
    pub fn get(&self, key: &str) -> Option<Sourced<toml::Value>> {
        let layer = Layer {
            mascot: Some(self.mascot.value.clone()),
            label: Some(self.label.value.clone()),
            label_case: Some(self.label_case.value),
            label_size: Some(self.label_size.value),
            size: Some(self.size.value),
            position: Some(self.position.value),
            offset: Some(self.offset.value),
            display: Some(self.display.value.clone()),
            animation: Some(self.animation.value),
            interval: Some(self.interval.value),
            accent: Some(self.accent.value.clone()),
            tray: Some(self.tray.value),
            fullscreen: Some(self.fullscreen.value),
            backend: Some(self.backend.value),
        };
        let source = match key {
            "mascot" => &self.mascot.source,
            "label" => &self.label.source,
            "label_case" => &self.label_case.source,
            "label_size" => &self.label_size.source,
            "size" => &self.size.source,
            "position" => &self.position.source,
            "offset" => &self.offset.source,
            "display" => &self.display.source,
            "animation" => &self.animation.source,
            "interval" => &self.interval.source,
            "accent" => &self.accent.source,
            "tray" => &self.tray.source,
            "fullscreen" => &self.fullscreen.source,
            "backend" => &self.backend.source,
            _ => return None,
        };
        Some(Sourced {
            value: layer.get(key)?,
            source: source.clone(),
        })
    }
}

/// Where an effective value came from, highest precedence first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Flag,
    /// A `[hosts]` table: the exact short name or a glob.
    Host(String),
    TopLevel,
    /// The state file (dragged position).
    StateFile,
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Flag => f.write_str("command-line flag"),
            Source::Host(p) => write!(f, "config [hosts.\"{p}\"]"),
            Source::TopLevel => f.write_str("config"),
            Source::StateFile => f.write_str("dragged; saved in state file"),
            Source::Default => f.write_str("default"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: Source,
}

/// Every setting after precedence is applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub mascot: Sourced<String>,
    pub label: Sourced<String>,
    pub label_case: Sourced<LabelCase>,
    pub label_size: Sourced<LabelSize>,
    pub size: Sourced<u32>,
    pub position: Sourced<Anchor>,
    pub offset: Sourced<[i32; 2]>,
    pub display: Sourced<String>,
    pub animation: Sourced<Animation>,
    pub interval: Sourced<[u32; 2]>,
    pub accent: Sourced<String>,
    pub tray: Sourced<bool>,
    pub fullscreen: Sourced<Fullscreen>,
    pub backend: Sourced<Backend>,
}

/// Every key a config file (or a `[hosts]` table) may set, in SPEC §8.2 order.
pub const KEYS: [&str; 14] = [
    "mascot",
    "label",
    "label_case",
    "label_size",
    "size",
    "position",
    "offset",
    "display",
    "animation",
    "interval",
    "accent",
    "tray",
    "fullscreen",
    "backend",
];

/// Check one `key = value` exactly as loading does. `Err` is the warning
/// loading would log for it.
pub fn check_value(key: &str, value: &toml::Value) -> Result<(), String> {
    let mut warnings = Vec::new();
    set_key(&mut Layer::default(), key, value, "", &mut warnings);
    warnings.pop().map_or(Ok(()), Err)
}

fn set_key(layer: &mut Layer, key: &str, v: &toml::Value, ctx: &str, warnings: &mut Vec<String>) {
    let mut bad = |expected: &str| {
        warnings.push(format!(
            "{ctx}{key} = {v}: expected {expected}; using the next layer down"
        ))
    };
    let s = v.as_str();
    match key {
        "mascot" => match s {
            Some(id) if id == "auto" || pawprint_sprite::is_valid_id(id) => {
                layer.mascot = Some(id.into())
            }
            _ => bad("\"auto\" or a mascot id"),
        },
        "label" => match s {
            Some(text) => layer.label = Some(text.into()),
            None => bad("a string"),
        },
        "label_case" => match s.and_then(LabelCase::parse) {
            Some(c) => layer.label_case = Some(c),
            None => bad("\"upper\", \"lower\", or \"as-is\""),
        },
        "label_size" => match s.and_then(LabelSize::parse) {
            Some(c) => layer.label_size = Some(c),
            None => bad("\"auto\", \"small\", \"medium\", or \"large\""),
        },
        "size" => match v.as_integer() {
            Some(n) if n > 0 && n % 32 == 0 && n <= 1024 => layer.size = Some(n as u32),
            _ => bad("a positive multiple of 32 (32, 64, 128, ...)"),
        },
        "position" => match s.and_then(Anchor::parse) {
            Some(a) => layer.position = Some(a),
            None => bad("an anchor such as \"top-right\""),
        },
        "offset" => match int_pair(v) {
            Some([x, y])
                if [x, y]
                    .iter()
                    .all(|n| (-MAX_OFFSET..=MAX_OFFSET).contains(n)) =>
            {
                layer.offset = Some([x as i32, y as i32])
            }
            _ => bad(&format!(
                "[x, y] logical px, each -{MAX_OFFSET} to {MAX_OFFSET}"
            )),
        },
        "display" => match s {
            Some(d) => layer.display = Some(d.into()),
            None => bad("\"primary\" or an output name"),
        },
        "animation" => match s.and_then(Animation::parse) {
            Some(a) => layer.animation = Some(a),
            None => bad("\"idle\" or \"static\""),
        },
        "interval" => match int_pair(v) {
            Some([lo, hi]) if 1 <= lo && lo <= hi && hi <= 3600 => {
                layer.interval = Some([lo as u32, hi as u32])
            }
            _ => bad("[min, max] seconds with 1 <= min <= max <= 3600"),
        },
        "accent" => match s {
            Some(a)
                if a == "auto"
                    || pawprint_sprite::Rgb::parse_hex(a).is_some()
                    || a.bytes().all(|b| b.is_ascii_lowercase()) =>
            {
                layer.accent = Some(a.into());
            }
            _ => bad("\"auto\", an accent name, or \"#rrggbb\""),
        },
        "tray" => match v.as_bool() {
            Some(b) => layer.tray = Some(b),
            None => bad("true or false"),
        },
        "fullscreen" => match s.and_then(Fullscreen::parse) {
            Some(f) => layer.fullscreen = Some(f),
            None => bad("\"hide\" or \"show\""),
        },
        "backend" => match s.and_then(Backend::parse) {
            Some(b) => layer.backend = Some(b),
            None => bad("\"auto\", \"windows\", \"macos\", \"x11\", or \"wayland\""),
        },
        _ => warnings.push(format!("{ctx}unknown key `{key}` ignored")),
    }
}

fn int_pair(v: &toml::Value) -> Option<[i64; 2]> {
    match v.as_array()?.as_slice() {
        [a, b] => Some([a.as_integer()?, b.as_integer()?]),
        _ => None,
    }
}

fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// A `[hosts]` key as it is matched: cut at the first `.`, as the short
/// name is, so `web01.example.com` matches host `web01` (SPEC §8.3).
fn host_pattern(key: &str) -> &str {
    key.split('.').next().unwrap_or(key)
}

/// Whether `[hosts."<key>"]` is the exact (non-glob) table for host `short`.
/// Case-insensitive. When several match, the last one in the file applies.
pub fn is_exact_host(key: &str, short: &str) -> bool {
    let p = host_pattern(key);
    !is_glob(p) && p.to_lowercase() == short.to_lowercase()
}

/// `*` matches any run of characters (including none), `?` exactly one.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        let cases = [
            ("prod-*", "prod-owl", true),
            ("prod-*", "prod-", true),
            ("prod-*", "dev-owl", false),
            ("*-db-??", "prod-db-01", true),
            ("*-db-??", "prod-db-1", false),
            ("*", "", true),
            ("a*b*c", "axxbyyc", true),
            ("a*b*c", "axxbyy", false),
            ("owl", "owl", true),
        ];
        for (p, t, want) in cases {
            assert_eq!(glob_match(p, t), want, "{p} vs {t}");
        }
    }

    #[test]
    fn defaults_with_no_file() {
        let s = Config::parse("").effective("owl", &Layer::default());
        assert_eq!(
            s.size,
            Sourced {
                value: 64,
                source: Source::Default
            }
        );
        assert_eq!(s.mascot.value, "auto");
        assert_eq!(s.position.value, Anchor::TopRight);
        assert_eq!(s.offset.value, [16, 16]);
        assert_eq!(s.interval.value, [8, 20]);
        assert!(s.tray.value);
        assert_eq!(s.fullscreen.value, Fullscreen::Hide);
    }

    const FLEET: &str = r#"
size = 32
accent = "teal"

[hosts."prod-*"]
accent = "cranberry"
label_case = "lower"

[hosts."prod-db-*"]
accent = "plum"

[hosts."*-db-*"]
accent = "brick"
size = 128

[hosts.prod-db-01]
mascot = "owl"

[hosts.buildbox]
mascot = "capybara"
"#;

    #[test]
    fn precedence() {
        let cfg = Config::parse(FLEET);
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        let s = cfg.effective("prod-db-01", &Layer::default());
        assert_eq!(
            s.mascot,
            Sourced {
                value: "owl".into(),
                source: Source::Host("prod-db-01".into())
            },
            "exact match"
        );
        assert_eq!(
            s.accent,
            Sourced {
                value: "plum".into(),
                source: Source::Host("prod-db-*".into())
            },
            "longest glob wins"
        );
        assert_eq!(
            s.size,
            Sourced {
                value: 128,
                source: Source::Host("*-db-*".into())
            },
            "less specific glob still applies"
        );
        assert_eq!(s.label_case.value, LabelCase::Lower);

        let s = cfg.effective("prod-web", &Layer::default());
        assert_eq!(s.accent.value, "cranberry");
        assert_eq!(
            s.size,
            Sourced {
                value: 32,
                source: Source::TopLevel
            }
        );

        let s = cfg.effective("BUILDBOX", &Layer::default());
        assert_eq!(s.mascot.value, "capybara", "host names compare lowercased");

        let flags = Layer {
            size: Some(256),
            ..Layer::default()
        };
        assert_eq!(
            cfg.effective("prod-db-01", &flags).size,
            Sourced {
                value: 256,
                source: Source::Flag
            }
        );
    }

    #[test]
    fn equal_length_globs_prefer_fewer_wildcards_then_file_order() {
        let cfg = Config::parse(
            "[hosts.\"ab*\"]\naccent = \"one\"\n[hosts.\"a**\"]\naccent = \"two\"\n[hosts.\"a?c\"]\naccent = \"three\"\n[hosts.\"?bc\"]\naccent = \"four\"",
        );
        assert_eq!(cfg.effective("abc", &Layer::default()).accent.value, "one");
        let cfg = Config::parse(
            "[hosts.\"a?c\"]\naccent = \"three\"\n[hosts.\"?bc\"]\naccent = \"four\"",
        );
        assert_eq!(
            cfg.effective("abc", &Layer::default()).accent.value,
            "three"
        );
    }

    #[test]
    fn warnings_never_fail() {
        let cfg = Config::parse(
            "size = 50\nshiny = true\nposition = \"middle\"\nlabel = \"LAB\"\ninterval = [20, 8]\noffset = [1]\n[hosts.x]\nwobble = 1\nmascot = \"Not An Id\"",
        );
        assert_eq!(cfg.warnings.len(), 7, "{:#?}", cfg.warnings);
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("unknown key `shiny`"))
        );
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("[hosts.\"x\"] unknown key `wobble`"))
        );
        let s = cfg.effective("x", &Layer::default());
        assert_eq!(s.size.value, 64, "invalid size falls back");
        assert_eq!(s.label.value, "LAB", "valid keys still apply");
        assert_eq!(s.mascot.value, "auto");

        let broken = Config::parse("size = [");
        assert_eq!(broken.warnings.len(), 1);
        assert_eq!(broken.effective("x", &Layer::default()).size.value, 64);
    }

    #[test]
    fn every_key_parses() {
        let cfg = Config::parse(
            "mascot = \"owl\"\nlabel = \"none\"\nlabel_case = \"as-is\"\nlabel_size = \"large\"\nsize = 128\nposition = \"bottom-left\"\noffset = [0, -4]\ndisplay = \"DP-1\"\nanimation = \"static\"\ninterval = [5, 5]\naccent = \"#112233\"\ntray = false\nfullscreen = \"show\"\nbackend = \"x11\"",
        );
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        let t = &cfg.top;
        assert_eq!(t.label_size, Some(LabelSize::Large));
        assert_eq!(t.position, Some(Anchor::BottomLeft));
        assert_eq!(t.offset, Some([0, -4]));
        assert_eq!(t.animation, Some(Animation::Static));
        assert_eq!(t.backend, Some(Backend::X11));
        assert_eq!(t.tray, Some(false));
        assert_eq!(t.fullscreen, Some(Fullscreen::Show));
        assert!(check_value("fullscreen", &toml::Value::from("cover")).is_err());
    }

    #[test]
    fn offset_is_range_checked_and_falls_through() {
        let cfg = Config::parse(
            "offset = [4, 5]\n[hosts.a]\noffset = [100001, 0]\n[hosts.b]\noffset = [0, -9999999999]\n[hosts.c]\noffset = [-100000, 100000]",
        );
        assert_eq!(cfg.warnings.len(), 2, "{:#?}", cfg.warnings);
        assert!(
            cfg.warnings
                .iter()
                .all(|w| w.contains(&format!("-{MAX_OFFSET} to {MAX_OFFSET}"))),
            "{:#?}",
            cfg.warnings
        );
        assert_eq!(cfg.effective("a", &Layer::default()).offset.value, [4, 5]);
        assert_eq!(cfg.effective("b", &Layer::default()).offset.value, [4, 5]);
        assert_eq!(
            cfg.effective("c", &Layer::default()).offset.value,
            [-100000, 100000]
        );
    }

    #[test]
    fn interval_warning_names_the_cap() {
        let cfg = Config::parse("interval = [1, 3601]");
        assert!(
            cfg.warnings[0].contains("max <= 3600"),
            "{:?}",
            cfg.warnings
        );
    }

    #[test]
    fn host_keys_match_the_short_name_of_an_fqdn() {
        let cfg = Config::parse(
            "[hosts.\"web01.example.com\"]\nmascot = \"owl\"\n[hosts.\"DB*.Example.com\"]\naccent = \"plum\"",
        );
        let s = cfg.effective("web01", &Layer::default());
        assert_eq!(
            s.mascot,
            Sourced {
                value: "owl".into(),
                source: Source::Host("web01.example.com".into())
            }
        );
        assert_eq!(
            cfg.effective("db02", &Layer::default()).accent.value,
            "plum"
        );
        assert_eq!(
            cfg.effective("web02", &Layer::default()).mascot.value,
            "auto"
        );
    }

    #[test]
    fn values_the_catalog_rejects_fall_through_to_the_next_layer() {
        let cfg = Config::parse(
            "mascot = \"owl\"\naccent = \"teal\"\n[hosts.\"x*\"]\nmascot = \"nope\"\naccent = \"mauve\"\n[hosts.x]\nmascot = \"gone\"",
        );
        let (s, warnings) = cfg.effective_checked(
            "x",
            &Layer::default(),
            |id| id == "owl",
            |name| name == "teal",
        );
        assert_eq!(s.mascot.value, "owl");
        assert_eq!(s.mascot.source, Source::TopLevel);
        assert_eq!(s.accent.value, "teal");
        assert_eq!(warnings.len(), 3, "{warnings:#?}");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("\"gone\"") && w.contains("[hosts.\"x\"]"))
        );
        assert!(
            warnings.iter().all(|w| w.contains("next layer down")),
            "{warnings:#?}"
        );
        let (s, _) = cfg.effective_checked("y", &Layer::default(), |_| false, |_| false);
        assert_eq!(
            (s.mascot.value.as_str(), s.accent.value.as_str()),
            ("auto", "auto"),
            "nothing valid: the default"
        );
    }

    #[test]
    fn every_key_reads_back_as_toml() {
        let text = "mascot = \"owl\"\nlabel = \"none\"\nlabel_case = \"as-is\"\nlabel_size = \"large\"\nsize = 128\nposition = \"bottom-left\"\noffset = [0, -4]\ndisplay = \"DP-1\"\nanimation = \"static\"\ninterval = [5, 9]\naccent = \"#112233\"\ntray = false\nfullscreen = \"show\"\nbackend = \"x11\"";
        let cfg = Config::parse(text);
        let table: toml::Table = text.parse().unwrap();
        let s = cfg.effective("x", &Layer::default());
        for key in KEYS {
            assert_eq!(cfg.top.get(key).as_ref(), table.get(key), "{key}");
            let got = s.get(key).unwrap();
            assert_eq!(Some(&got.value), table.get(key), "{key}");
            assert_eq!(got.source, Source::TopLevel, "{key}");
            assert!(check_value(key, &got.value).is_ok(), "{key}");
        }
        assert_eq!(cfg.top.get("shiny"), None);
        assert_eq!(Layer::default().get("size"), None);
        let defaults = Config::default().effective("x", &Layer::default());
        assert_eq!(
            defaults.get("offset").unwrap().value.to_string(),
            "[16, 16]"
        );
        assert_eq!(defaults.get("fullscreen").unwrap().source, Source::Default);
    }

    #[test]
    fn host_keys_keep_their_spelling() {
        let cfg = Config::parse("[hosts.BuildBox]\nsize = 32\n[hosts.\"Prod-*\"]\nsize = 64");
        assert_eq!(cfg.host_keys, ["BuildBox", "Prod-*"]);
        assert_eq!(cfg.hosts[0].0, "buildbox");
    }

    #[test]
    fn duplicate_host_tables_the_last_wins() {
        let cfg = Config::parse("[hosts.OWL]\nsize = 32\n[hosts.owl]\nsize = 128");
        assert_eq!(
            cfg.effective("owl", &Layer::default()).size.source,
            Source::Host("owl".into())
        );
        assert_eq!(cfg.effective("owl", &Layer::default()).size.value, 128);
    }
}
