//! Who is this host? Hostname + config + state + catalog → label, mascot,
//! accent, and every effective setting, with the reasons `pawprint explain`
//! prints (SPEC §5).

use std::fmt;

use pawprint_sprite::Rgb;

use crate::accent::{self, Accent};
use crate::catalog::Catalog;
use crate::config::{Config, Layer, Settings, Source, Sourced};
use crate::hash::hrw_pick;
use crate::host::{self, LabelSetting};
use crate::layout::Position;
use crate::matching::{self, TokenMatch};
use crate::state::State;

/// The mascot rule that fired (SPEC §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Explicit = 1,
    Cached = 2,
    WholeName = 3,
    Token = 4,
    Fallback = 5,
}

impl Rule {
    pub const ALL: [Rule; 5] = [
        Rule::Explicit,
        Rule::Cached,
        Rule::WholeName,
        Rule::Token,
        Rule::Fallback,
    ];

    pub fn number(self) -> u8 {
        self as u8
    }

    pub fn title(self) -> &'static str {
        match self {
            Rule::Explicit => "explicit override",
            Rule::Cached => "cached assignment",
            Rule::WholeName => "whole-name alias",
            Rule::Token => "token match",
            Rule::Fallback => "deterministic fallback",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rule {}: {}", self.number(), self.title())
    }
}

/// What one rule found, for `pawprint explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleStep {
    pub rule: Rule,
    pub outcome: String,
}

/// Everything decided about this host.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub raw: String,
    pub short: String,
    pub settings: Settings,
    pub position: Sourced<Position>,
    pub label: Option<String>,
    pub mascot: String,
    pub rule: Rule,
    /// One step per rule that was evaluated, in order; the last one won.
    pub trace: Vec<RuleStep>,
    pub accent: Accent,
    /// A new rule-5 result the caller should cache in the state file.
    pub new_assignment: Option<String>,
    pub warnings: Vec<String>,
}

/// Resolve the host's identity. With `recompute`, the cache (rule 2) is
/// skipped to show what the hash would choose today.
pub fn resolve(
    hostname: &str,
    config: &Config,
    state: &State,
    flags: &Layer,
    catalog: &Catalog,
    accents: &[(String, Rgb)],
    recompute: bool,
) -> Resolved {
    let raw = hostname.trim().to_string();
    let short = host::short_name(&raw);
    let (settings, skipped) = config.effective_checked(
        &short,
        flags,
        |id| catalog.get(id).is_some(),
        |name| accent::lookup(name, accents).is_some(),
    );
    let mut warnings = config.warnings.clone();
    warnings.extend(skipped);
    let mut trace = Vec::new();
    let mut step = |rule: Rule, outcome: String| trace.push(RuleStep { rule, outcome });

    let chosen = choose(
        &short,
        &settings.mascot.value,
        state,
        catalog,
        recompute,
        &mut step,
    );
    let (mascot, rule) = chosen;
    let new_assignment =
        (rule == Rule::Fallback && !recompute && state.assignments.get(&short) != Some(&mascot))
            .then(|| mascot.clone());

    let mascot_accent = catalog
        .get(&mascot)
        .map(|m| m.accent.as_str())
        .unwrap_or("auto");
    let fallback = matches!(rule, Rule::Cached | Rule::Fallback);
    let (accent, warning) = accent::resolve(
        &settings.accent.value,
        fallback,
        mascot_accent,
        &short,
        accents,
    );
    warnings.extend(warning);

    let label = host::label(
        &raw,
        &LabelSetting::parse(&settings.label.value),
        settings.label_case.value,
    );
    let position = match state.position {
        Some(p) => Sourced {
            value: p,
            source: Source::StateFile,
        },
        None => Sourced {
            value: Position {
                anchor: settings.position.value,
                offset: settings.offset.value,
            },
            source: if settings.position.source == Source::Default {
                settings.offset.source.clone()
            } else {
                settings.position.source.clone()
            },
        },
    };
    Resolved {
        raw,
        short,
        settings,
        position,
        label,
        mascot,
        rule,
        trace,
        accent,
        new_assignment,
        warnings,
    }
}

fn choose(
    short: &str,
    explicit: &str,
    state: &State,
    catalog: &Catalog,
    recompute: bool,
    step: &mut impl FnMut(Rule, String),
) -> (String, Rule) {
    // Rule 1. `explicit` is "auto" or in the catalog: precedence skips
    // anything else (`Config::effective_checked`).
    if explicit == "auto" {
        step(Rule::Explicit, "none (mascot = \"auto\")".into());
    } else {
        step(Rule::Explicit, format!("mascot = {explicit:?}"));
        return (explicit.to_string(), Rule::Explicit);
    }

    // Rule 2
    match state.assignments.get(short) {
        _ if recompute => step(Rule::Cached, "skipped (--recompute)".into()),
        Some(id) if catalog.get(id).is_some() => {
            step(Rule::Cached, format!("{id} (state file)"));
            return (id.clone(), Rule::Cached);
        }
        Some(id) => step(
            Rule::Cached,
            format!("{id} is no longer in the catalog, ignored"),
        ),
        None => step(Rule::Cached, "none".into()),
    }

    // Rule 3
    match matching::whole_name(short, catalog.aliases()) {
        Some((name, id)) => {
            step(Rule::WholeName, format!("{name:?} → {id}"));
            return (id, Rule::WholeName);
        }
        None => step(
            Rule::WholeName,
            format!("no alias {:?}", matching::tokens(short).concat()),
        ),
    }

    // Rule 4
    match matching::token_match(short, catalog.aliases()) {
        TokenMatch::One { ngram, id } => {
            step(Rule::Token, format!("token {ngram:?} → {id}"));
            return (id, Rule::Token);
        }
        TokenMatch::Ambiguous(hits) => {
            let list: Vec<String> = hits.iter().map(|(t, id)| format!("{t:?} → {id}")).collect();
            step(
                Rule::Token,
                format!("ambiguous ({}), the hash decides", list.join(", ")),
            );
        }
        TokenMatch::None => step(
            Rule::Token,
            format!("no token of {:?} names a mascot", matching::tokens(short)),
        ),
    }

    // Rule 5
    let pick = hrw_pick(short, catalog.eligible())
        .or_else(|| catalog.mascots().first().map(|m| m.id.as_str()));
    let id = pick.expect("the catalog always has a mascot").to_string();
    let n = catalog.eligible().count();
    step(
        Rule::Fallback,
        format!(
            "{id} (rendezvous hash over {n} mascot{})",
            if n == 1 { "" } else { "s" }
        ),
    );
    (id, Rule::Fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::bundle;
    use pawprint_sprite::{Action, Mascot, When};

    fn m(id: &str, aliases: &[&str], accent: &str) -> Mascot {
        Mascot {
            id: id.into(),
            name: id.into(),
            category: "mammals".into(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            accent: accent.into(),
            fallback: true,
            nocturnal: false,
            base: None,
            colors: vec![],
            frames: vec![vec![0; 1024]],
            actions: vec![Action {
                name: "a".into(),
                weight: 1,
                when: When::Any,
                template: None,
                frames: vec![],
            }],
        }
    }

    fn catalog() -> Catalog {
        let mut never = m("ghost", &[], "ice");
        never.fallback = false;
        Catalog::new(vec![
            m("owl", &["hoot"], "violet"),
            m("bear", &[], "cranberry"),
            m("moose", &[], "pine"),
            m("red-panda", &[], "brick"),
            m("capybara", &[], "honey"),
            never,
        ])
    }

    fn run(host: &str, config: &str, state: &State) -> Resolved {
        resolve(
            host,
            &Config::parse(config),
            state,
            &Layer::default(),
            &catalog(),
            &bundle().accents,
            false,
        )
    }

    #[test]
    fn rules_table() {
        let cases = [
            ("owl", "", "owl", Rule::WholeName),
            ("OWL.lab.example", "", "owl", Rule::WholeName),
            ("red-panda-01", "", "red-panda", Rule::WholeName),
            ("prod-owl-02.lab.example", "", "owl", Rule::Token),
            ("prod-owl02", "", "owl", Rule::Token),
            ("snowless", "", "", Rule::Fallback),
            ("bear-moose", "", "", Rule::Fallback),
            ("owl", "mascot = \"bear\"", "bear", Rule::Explicit),
            (
                "prod-db-01",
                "[hosts.\"prod-*\"]\nmascot = \"moose\"",
                "moose",
                Rule::Explicit,
            ),
        ];
        for (host, config, want, rule) in cases {
            let r = run(host, config, &State::default());
            assert_eq!(r.rule, rule, "{host}");
            if !want.is_empty() {
                assert_eq!(r.mascot, want, "{host}");
            }
            assert_eq!(r.trace.last().unwrap().rule, rule);
            assert_eq!(
                r.trace.len(),
                rule.number() as usize,
                "every earlier rule is traced"
            );
        }
    }

    #[test]
    fn fallback_is_deterministic_cached_and_never_picks_ineligible() {
        let r = run("buildbox", "", &State::default());
        assert_eq!(r.rule, Rule::Fallback);
        assert_ne!(r.mascot, "ghost");
        assert_eq!(r.new_assignment.as_deref(), Some(r.mascot.as_str()));
        assert_eq!(r.mascot, run("buildbox", "", &State::default()).mascot);

        let mut state = State::default();
        state
            .assignments
            .insert("buildbox".into(), "capybara".into());
        let cached = run("buildbox", "", &state);
        assert_eq!(
            (cached.mascot.as_str(), cached.rule, cached.new_assignment),
            ("capybara", Rule::Cached, None)
        );

        let recomputed = resolve(
            "buildbox",
            &Config::default(),
            &state,
            &Layer::default(),
            &catalog(),
            &bundle().accents,
            true,
        );
        assert_eq!(
            (recomputed.rule, recomputed.new_assignment),
            (Rule::Fallback, None)
        );

        state.assignments.insert("buildbox".into(), "dodo".into());
        assert_eq!(
            run("buildbox", "", &state).rule,
            Rule::Fallback,
            "stale cache entries are ignored"
        );
    }

    #[test]
    fn unknown_explicit_mascot_warns_and_falls_through() {
        let r = run("owl", "mascot = \"dodo\"", &State::default());
        assert_eq!((r.mascot.as_str(), r.rule), ("owl", Rule::WholeName));
        assert!(r.warnings.iter().any(|w| w.contains("dodo")));
    }

    #[test]
    fn invalid_host_values_fall_back_to_the_next_layer() {
        let config = "mascot = \"bear\"\naccent = \"teal\"\n[hosts.owl]\nmascot = \"dodo\"\naccent = \"mauve\"";
        let r = run("owl", config, &State::default());
        assert_eq!((r.mascot.as_str(), r.rule), ("bear", Rule::Explicit));
        assert_eq!(r.settings.mascot.source, Source::TopLevel);
        assert_eq!(r.accent.name, "teal");
        assert!(r.warnings.iter().any(|w| w.contains("\"dodo\"")));
        assert!(r.warnings.iter().any(|w| w.contains("\"mauve\"")));
    }

    #[test]
    fn accents_follow_the_rule() {
        assert_eq!(run("owl", "", &State::default()).accent.name, "violet");
        assert_eq!(
            run("owl", "accent = \"teal\"", &State::default())
                .accent
                .name,
            "teal"
        );
        let fb = run("buildbox", "", &State::default());
        assert_eq!(fb.accent.source, accent::AccentSource::Fallback);
    }

    #[test]
    fn label_and_position() {
        let r = run("prod-owl-02.lab.example", "", &State::default());
        assert_eq!(r.short, "prod-owl-02");
        assert_eq!(r.label.as_deref(), Some("PROD-OWL-02"));
        assert_eq!(r.position.value, Position::default());
        assert_eq!(r.position.source, Source::Default);

        let r = run(
            "owl",
            "label = \"none\"\nposition = \"bottom-left\"",
            &State::default(),
        );
        assert_eq!(r.label, None);
        assert_eq!(r.position.source, Source::TopLevel);

        let state = State {
            position: Some(Position {
                anchor: crate::layout::Anchor::Center,
                offset: [0, 0],
            }),
            ..State::default()
        };
        let r = run("owl", "position = \"bottom-left\"", &state);
        assert_eq!(
            (r.position.value, r.position.source),
            (state.position.unwrap(), Source::StateFile)
        );
    }
}
