//! Hostname → short name → label (SPEC §5.1–5.2, §4.4).

use std::fmt;

/// Labels longer than this keep only their first this-many characters (SPEC §4.4).
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

/// The text drawn on the crate, or `None` for no label. A label from the
/// hostname keeps only its letters and digits (`prod-owl-02` → `prodowl02`),
/// so more of it fits on the crate; literal text is kept as written.
pub fn label(raw: &str, setting: &LabelSetting, case: LabelCase) -> Option<String> {
    let text = match setting {
        LabelSetting::Auto => alphanumeric(&short_name(raw)),
        LabelSetting::Full => alphanumeric(&raw.trim().to_lowercase()),
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

fn alphanumeric(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// The first [`MAX_LABEL`] characters. The left side is what the strip
/// shows when even these don't all fit (SPEC §4.4), so it is what is kept.
pub fn truncate(s: &str) -> String {
    s.chars().take(MAX_LABEL).collect()
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
        let text = |t: &str| LabelSetting::Text(t.into());
        let cases = [
            ("owl.lab.example", LabelSetting::Auto, Upper, Some("OWL")),
            // From the hostname: letters and digits only.
            (
                "OWL-02.LAB.EXAMPLE",
                LabelSetting::Auto,
                AsIs,
                Some("owl02"),
            ),
            ("prod-owl-02", LabelSetting::Auto, Upper, Some("PRODOWL02")),
            ("build_box", LabelSetting::Auto, Upper, Some("BUILDBOX")),
            (
                "owl.lab.example",
                LabelSetting::Full,
                Upper,
                Some("OWLLABEXAMPLE"),
            ),
            (
                "Owl-02.lab.example",
                LabelSetting::Full,
                AsIs,
                Some("owl02labexample"),
            ),
            ("owl", LabelSetting::None, Upper, None),
            // Literal text is kept as written, punctuation and all.
            ("owl", text("Build-7"), AsIs, Some("Build-7")),
            ("owl", text("Build-7"), Lower, Some("build-7")),
            ("owl", text("Build-7"), Upper, Some("BUILD-7")),
            ("owl", text("a.b_c d"), AsIs, Some("a.b_c d")),
            // 20 characters at most, counted after stripping, from the left.
            (
                "jenkins-agent-linux-07",
                LabelSetting::Auto,
                Upper,
                Some("JENKINSAGENTLINUX07"),
            ),
            (
                "prod-database-primary-01",
                LabelSetting::Auto,
                Upper,
                Some("PRODDATABASEPRIMARY0"),
            ),
            (
                "owl",
                text("prod-database-primary-01"),
                Upper,
                Some("PROD-DATABASE-PRIMAR"),
            ),
            (long.as_str(), LabelSetting::Auto, Lower, Some(&long[..20])),
            ("", LabelSetting::Auto, Upper, None),
            // Nothing left after stripping: no label.
            ("---", LabelSetting::Auto, Upper, None),
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
    fn truncation_keeps_the_left_side() {
        assert_eq!(truncate(&"x".repeat(20)), "x".repeat(20));
        assert_eq!(truncate(&"x".repeat(21)), "x".repeat(20));
        assert_eq!(
            truncate("gkeproddefaultpool1a2b3c4dxyz9"),
            "gkeproddefaultpool1a"
        );
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
