//! Hostname → short name → label (SPEC §5.1–5.2, §4.4).

use std::fmt;

/// Labels longer than this are cut and end in `..` (SPEC §4.4; provisional,
/// measured in M2).
pub const MAX_LABEL: usize = 20;

/// `raw` lowercased and cut at the first `.`: `owl-02.lab.example` → `owl-02`.
pub fn short_name(raw: &str) -> String {
    let raw = raw.trim().to_lowercase();
    raw.split('.').next().unwrap_or_default().to_string()
}

/// The `label` setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelSetting {
    /// The short hostname.
    Auto,
    /// The full hostname, lowercased.
    Full,
    /// No label at all.
    None,
    /// Literal text.
    Text(String),
}

impl LabelSetting {
    pub fn parse(s: &str) -> LabelSetting {
        match s {
            "auto" => LabelSetting::Auto,
            "full" => LabelSetting::Full,
            "none" => LabelSetting::None,
            text => LabelSetting::Text(text.to_string()),
        }
    }
}

/// The `label_case` setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LabelCase {
    #[default]
    Upper,
    Lower,
    AsIs,
}

impl LabelCase {
    pub fn parse(s: &str) -> Option<LabelCase> {
        match s {
            "upper" => Some(LabelCase::Upper),
            "lower" => Some(LabelCase::Lower),
            "as-is" => Some(LabelCase::AsIs),
            _ => None,
        }
    }
}

impl fmt::Display for LabelCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LabelCase::Upper => "upper",
            LabelCase::Lower => "lower",
            LabelCase::AsIs => "as-is",
        })
    }
}

/// The text drawn on the crate, or `None` for no label.
pub fn label(raw: &str, setting: &LabelSetting, case: LabelCase) -> Option<String> {
    let text = match setting {
        LabelSetting::Auto => short_name(raw),
        LabelSetting::Full => raw.trim().to_lowercase(),
        LabelSetting::None => return None,
        LabelSetting::Text(t) => t.clone(),
    };
    let text = match case {
        LabelCase::Upper => text.to_uppercase(),
        LabelCase::Lower => text.to_lowercase(),
        LabelCase::AsIs => text,
    };
    let text = truncate(&text);
    (!text.is_empty()).then_some(text)
}

/// Cut to [`MAX_LABEL`] characters, ending in `..` when cut.
pub fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_LABEL {
        s.to_string()
    } else {
        s.chars().take(MAX_LABEL - 2).chain("..".chars()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names() {
        let cases = [
            ("owl", "owl"),
            ("owl.lab.example", "owl"),
            ("OWL-02.LAB.EXAMPLE", "owl-02"),
            ("  prod-owl02.local\n", "prod-owl02"),
            ("x", "x"),
            ("", ""),
        ];
        for (raw, want) in cases {
            assert_eq!(short_name(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn labels() {
        use LabelCase::*;
        let long = "a".repeat(63);
        let cases = [
            ("owl.lab.example", LabelSetting::Auto, Upper, Some("OWL")),
            (
                "OWL-02.LAB.EXAMPLE",
                LabelSetting::Auto,
                AsIs,
                Some("owl-02"),
            ),
            (
                "owl.lab.example",
                LabelSetting::Full,
                Upper,
                Some("OWL.LAB.EXAMPLE"),
            ),
            (
                "owl.lab.example",
                LabelSetting::Full,
                AsIs,
                Some("owl.lab.example"),
            ),
            ("owl", LabelSetting::None, Upper, None),
            (
                "owl",
                LabelSetting::Text("Build-7".into()),
                AsIs,
                Some("Build-7"),
            ),
            (
                "owl",
                LabelSetting::Text("Build-7".into()),
                Lower,
                Some("build-7"),
            ),
            (
                "prod-database-primary-01",
                LabelSetting::Auto,
                Upper,
                Some("PROD-DATABASE-PRIM.."),
            ),
            (
                long.as_str(),
                LabelSetting::Auto,
                Lower,
                Some("aaaaaaaaaaaaaaaaaa.."),
            ),
            ("", LabelSetting::Auto, Upper, None),
        ];
        for (raw, setting, case, want) in cases {
            assert_eq!(
                label(raw, &setting, case).as_deref(),
                want,
                "{raw:?} {setting:?} {case:?}"
            );
        }
    }

    #[test]
    fn truncation_boundary() {
        assert_eq!(truncate(&"x".repeat(20)), "x".repeat(20));
        assert_eq!(truncate(&"x".repeat(21)).chars().count(), 20);
        assert!(truncate(&"x".repeat(21)).ends_with(".."));
    }

    #[test]
    fn parse_settings() {
        assert_eq!(LabelSetting::parse("auto"), LabelSetting::Auto);
        assert_eq!(LabelSetting::parse("full"), LabelSetting::Full);
        assert_eq!(LabelSetting::parse("none"), LabelSetting::None);
        assert_eq!(LabelSetting::parse("Lab"), LabelSetting::Text("Lab".into()));
        assert_eq!(LabelCase::parse("as-is"), Some(LabelCase::AsIs));
        assert_eq!(LabelCase::parse("title"), None);
    }
}
