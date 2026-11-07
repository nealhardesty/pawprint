//! Name matching: `squash`, the tokenizer, the alias table, and rules 3 and
//! 4 of mascot selection (SPEC §5.3).

use std::collections::{BTreeSet, HashMap};

pub use pawprint_sprite::squash;

/// Split on every non-alphanumeric character and at every letter↔digit
/// boundary, then drop all-digit tokens: `prod-owl02` → `[prod, owl]`.
pub fn tokens(short: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_digit = false;
    for c in short.chars().flat_map(char::to_lowercase) {
        let alpha = c.is_ascii_lowercase();
        let digit = c.is_ascii_digit();
        if !(alpha || digit) || (!cur.is_empty() && digit != cur_digit) {
            out.push(std::mem::take(&mut cur));
        }
        if alpha || digit {
            cur.push(c);
            cur_digit = digit;
        }
    }
    out.push(cur);
    out.retain(|t| !t.is_empty() && !t.bytes().all(|b| b.is_ascii_digit()));
    out
}

/// Squashed alias → mascot id. Every id is an implicit alias of itself.
#[derive(Debug, Clone, Default)]
pub struct AliasTable {
    map: HashMap<String, String>,
}

impl AliasTable {
    /// Build from `(id, aliases)`. Later entries win a collision, so user
    /// mascots listed after bundled ones override them.
    pub fn new<'a>(entries: impl IntoIterator<Item = (&'a str, &'a [String])>) -> AliasTable {
        let mut map = HashMap::new();
        for (id, aliases) in entries {
            for alias in std::iter::once(id).chain(aliases.iter().map(String::as_str)) {
                map.insert(squash(alias), id.to_string());
            }
        }
        AliasTable { map }
    }

    pub fn lookup(&self, squashed: &str) -> Option<&str> {
        self.map.get(squashed).map(String::as_str)
    }
}

/// Rule 3: the whole name, digits dropped (`red-panda-01` → `redpanda`),
/// equals an alias. Returns `(squashed name, id)`.
pub fn whole_name(short: &str, table: &AliasTable) -> Option<(String, String)> {
    let name: String = tokens(short).concat();
    table.lookup(&name).map(|id| (name.clone(), id.to_string()))
}

/// The outcome of rule 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenMatch {
    /// Exactly one mascot at the longest matching n-gram length.
    One {
        ngram: String,
        id: String,
    },
    /// Several distinct mascots tie; the hash decides (rule 5).
    Ambiguous(Vec<(String, String)>),
    None,
}

/// Rule 4: for n from the token count down to 1, look up every run of n
/// adjacent tokens. The first n with any match decides.
pub fn token_match(short: &str, table: &AliasTable) -> TokenMatch {
    let toks = tokens(short);
    for n in (1..=toks.len()).rev() {
        let hits: Vec<(String, String)> = toks
            .windows(n)
            .filter_map(|w| {
                let ngram = w.concat();
                table
                    .lookup(&ngram)
                    .map(|id| (ngram.clone(), id.to_string()))
            })
            .collect();
        let ids: BTreeSet<&str> = hits.iter().map(|(_, id)| id.as_str()).collect();
        match ids.len() {
            0 => continue,
            1 => {
                let (ngram, id) = hits.into_iter().next().expect("one hit");
                return TokenMatch::One { ngram, id };
            }
            _ => return TokenMatch::Ambiguous(hits),
        }
    }
    TokenMatch::None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn table() -> AliasTable {
        let entries: Vec<(&str, Vec<String>)> = vec![
            ("owl", vec!["owls".into(), "hoot".into()]),
            ("polar-bear", vec![]),
            ("bear", vec!["grizzly".into()]),
            ("moose", vec![]),
            ("red-panda", vec!["firefox".into()]),
            ("cat-orange-tabby", vec!["cat".into(), "ginger".into()]),
        ];
        AliasTable::new(entries.iter().map(|(id, a)| (*id, a.as_slice())))
    }

    #[test]
    fn tokenizer() {
        let cases: [(&str, &[&str]); 9] = [
            ("prod-owl02", &["prod", "owl"]),
            ("prod-owl-02", &["prod", "owl"]),
            ("red-panda-01", &["red", "panda"]),
            ("owl2go", &["owl", "go"]),
            ("db01", &["db"]),
            ("1234", &[]),
            ("x", &["x"]),
            ("Web_Server.v2", &["web", "server", "v"]),
            ("", &[]),
        ];
        for (input, want) in cases {
            assert_eq!(tokens(input), want, "{input:?}");
        }
    }

    #[test]
    fn rule3_whole_name() {
        let t = table();
        let cases = [
            ("owl", Some("owl")),
            ("polar-bear", Some("polar-bear")),
            ("polarbear", Some("polar-bear")),
            ("red-panda-01", Some("red-panda")),
            ("hoot", Some("owl")),
            ("prod-owl", None),
            ("snowless", None),
        ];
        for (short, want) in cases {
            assert_eq!(
                whole_name(short, &t).map(|(_, id)| id).as_deref(),
                want,
                "{short}"
            );
        }
    }

    #[test]
    fn rule4_tokens() {
        let t = table();
        let one = |ngram: &str, id: &str| TokenMatch::One {
            ngram: ngram.into(),
            id: id.into(),
        };
        assert_eq!(token_match("prod-owl-02", &t), one("owl", "owl"));
        assert_eq!(token_match("prod-owl02", &t), one("owl", "owl"));
        assert_eq!(
            token_match("lab-polar-bear-3", &t),
            one("polarbear", "polar-bear"),
            "longest n-gram wins over `bear`"
        );
        assert_eq!(
            token_match("prod-red-panda", &t),
            one("redpanda", "red-panda")
        );
        assert_eq!(
            token_match("snowless", &t),
            TokenMatch::None,
            "whole words only"
        );
        assert_eq!(token_match("bearded", &t), TokenMatch::None);
        assert_eq!(token_match("buildbox", &t), TokenMatch::None);
        match token_match("bear-moose", &t) {
            TokenMatch::Ambiguous(hits) => assert_eq!(hits.len(), 2),
            other => panic!("bear-moose should be ambiguous, got {other:?}"),
        }
        assert_eq!(
            token_match("owl-hoot", &t),
            one("owl", "owl"),
            "two aliases of one mascot is not ambiguous"
        );
    }
}
