//! Editing `config.toml` without disturbing it (SPEC §8.6): `pawprint set`,
//! `pawprint unset`, and the tray's Size and Animation choices all write one
//! key into this host's exact `[hosts."<short>"]` table with `toml_edit`, so
//! comments, order, and formatting survive. Values are checked by the same
//! parser that loads the file ([`config::check_value`]).

use toml_edit::{Decor, DocumentMut, Item, Table, Value};

use crate::config::{self, KEYS};

/// What `pawprint config init` writes: every key with its default, as in
/// SPEC §8.2, so editing a value is all it takes.
pub const TEMPLATE: &str = r##"# PAWPRINT configuration. Every key is optional; the values below are the
# defaults. `pawprint explain` shows what is in effect and why.
# Reference: https://github.com/nealhardesty/pawprint/blob/master/SPEC.md#82-configtoml-reference

# Which mascot to show. "auto" picks one from the hostname (see `pawprint explain`).
mascot = "auto"            # "auto" | a mascot id from `pawprint list`

# Text on the crate.
label = "auto"             # "auto" (short hostname) | "full" | "none" | any text
label_case = "upper"       # "upper" | "lower" | "as-is"
label_size = "auto"        # "auto" | "small" | "medium" | "large"

# Mascot size in logical pixels. 32, 64, and 128 are the presets; any multiple of 32 works.
size = 64

# Where the badge sits on the target display. Dragging in move mode overrides this.
position = "top-right"     # top-left | top | top-right | left | center | right | bottom-left | bottom | bottom-right
offset = [16, 16]          # logical px inward from the anchor
display = "primary"        # "primary" | an output name from `pawprint doctor`

# Animation.
animation = "idle"         # "idle" | "static"
interval = [8, 20]         # seconds between idle actions (min, max)

# Crate color.
accent = "auto"            # "auto" | a palette name (see `pawprint list --accents`) | "#rrggbb"

# Tray icon (Windows), menu-bar item (macOS), or StatusNotifierItem (Linux).
tray = true

# Window backend. Leave on auto unless `pawprint doctor` says otherwise.
backend = "auto"           # "auto" | "windows" | "macos" | "x11" | "wayland"

# Per-host overrides. Keys are short hostnames or glob patterns (* and ?).
# Any top-level key may appear in a host table. `pawprint set <key> <value>`
# and the tray's Size and Animation menus add this host's table below.
#
# [hosts.buildbox]
# mascot = "capybara"
#
# [hosts."prod-*"]
# accent = "cranberry"
# label_case = "upper"
[hosts]
"##;

/// Turn a command-line value into a TOML value for `key`, checked the way
/// loading checks it. The text is read as TOML first (`64`, `false`,
/// `[8, 20]`, `"quoted"`) and otherwise as a plain string (`otter`), so
/// quotes are never needed on the command line.
pub fn parse_value(key: &str, raw: &str) -> Result<Value, String> {
    if !KEYS.contains(&key) {
        return Err(format!(
            "unknown key `{key}`; the keys are {}",
            KEYS.join(", ")
        ));
    }
    let typed = raw.parse::<Value>().ok();
    let mut first_error = None;
    for value in typed.into_iter().chain([Value::from(raw)]) {
        match config::check_value(key, &to_toml(key, &value)) {
            Ok(()) => {
                let mut value = value;
                *value.decor_mut() = Decor::default();
                return Ok(value);
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.expect("at least one attempt"))
}

/// The same value as the loader sees it.
fn to_toml(key: &str, value: &Value) -> toml::Value {
    let doc = format!("{key} = {}", value.clone().decorated("", ""));
    doc.parse::<toml::Table>()
        .ok()
        .and_then(|mut t| t.remove(key))
        .expect("a toml_edit value is valid TOML")
}

/// The `[hosts]` table's key for host `short`: an existing exact (non-glob)
/// entry that matches it case-insensitively, else `short` lowercased.
fn host_key(hosts: &Table, short: &str) -> String {
    let short = short.to_lowercase();
    hosts
        .iter()
        .map(|(k, _)| k)
        .filter(|k| !k.contains(['*', '?']))
        .find(|k| k.to_lowercase() == short)
        .map_or(short, str::to_string)
}

fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|e| format!("config.toml is not valid TOML, so it was left alone: {e}"))
}

/// Set `key = value` in `[hosts."<short>"]`, creating the table (and
/// `[hosts]`) if needed. A value already there keeps its trailing comment.
pub fn set_host_key(text: &str, short: &str, key: &str, value: Value) -> Result<String, String> {
    let mut doc = parse(text)?;
    let hosts = doc
        .entry("hosts")
        .or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        })
        .as_table_mut()
        .ok_or("`hosts` in config.toml is not a table, so it was left alone")?;
    let name = host_key(hosts, short);
    let host = hosts
        .entry(&name)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| format!("[hosts.{name:?}] in config.toml is not a table"))?;
    let decor = host
        .get(key)
        .and_then(Item::as_value)
        .map(|v| v.decor().clone());
    let mut value = value;
    if let Some(d) = decor {
        *value.decor_mut() = d;
    }
    host[key] = Item::Value(value);
    Ok(doc.to_string())
}

/// [`parse_value`] then [`set_host_key`]: what `pawprint set` and the tray
/// do.
pub fn set(text: &str, short: &str, key: &str, raw: &str) -> Result<String, String> {
    set_host_key(text, short, key, parse_value(key, raw)?)
}

/// Remove `key` from `[hosts."<short>"]`. Returns the new text and whether
/// the key was there. A host table left empty (and without comments) is
/// removed, and so is an empty implicit `[hosts]`.
pub fn unset_host_key(text: &str, short: &str, key: &str) -> Result<(String, bool), String> {
    let mut doc = parse(text)?;
    let Some(hosts) = doc.get_mut("hosts").and_then(Item::as_table_mut) else {
        return Ok((text.to_string(), false));
    };
    let name = host_key(hosts, short);
    let Some(host) = hosts.get_mut(&name).and_then(Item::as_table_mut) else {
        return Ok((text.to_string(), false));
    };
    if host.remove(key).is_none() {
        return Ok((text.to_string(), false));
    }
    let commented = host
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .is_some_and(|p| p.contains('#'));
    if host.is_empty() && !commented {
        hosts.remove(&name);
    }
    if hosts.is_empty() && hosts.is_implicit() {
        doc.remove("hosts");
    }
    Ok((doc.to_string(), true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Layer, Source};

    fn set(text: &str, short: &str, key: &str, raw: &str) -> String {
        super::set(text, short, key, raw).unwrap()
    }

    #[test]
    fn template_is_every_default() {
        let cfg = Config::parse(TEMPLATE);
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        assert!(cfg.hosts.is_empty(), "host examples are commented out");
        // A host's table goes after the examples, at the end.
        let out = super::set(TEMPLATE, "owl", "size", "128").unwrap();
        assert!(
            out.ends_with("label_case = \"upper\"\n[hosts]\n\n[hosts.owl]\nsize = 128\n"),
            "{out}"
        );
        assert_eq!(unset_host_key(&out, "owl", "size").unwrap().0, TEMPLATE);
        let from_file = cfg.effective("owl", &Layer::default());
        let defaults = Config::default().effective("owl", &Layer::default());
        // Same values; every one from the file.
        assert_eq!(
            format!("{:?}", from_file).replace("TopLevel", "Default"),
            format!("{defaults:?}")
        );
        for key in KEYS {
            assert!(
                TEMPLATE
                    .lines()
                    .any(|l| l.starts_with(&format!("{key} = "))),
                "{key} is in the template"
            );
        }
    }

    #[test]
    fn values_are_typed_and_checked_like_loading() {
        assert_eq!(parse_value("size", "128").unwrap().as_integer(), Some(128));
        assert_eq!(parse_value("tray", "false").unwrap().as_bool(), Some(false));
        assert_eq!(
            parse_value("mascot", "otter").unwrap().as_str(),
            Some("otter")
        );
        assert_eq!(parse_value("label", "1234").unwrap().as_str(), Some("1234"));
        assert_eq!(
            parse_value("label", "\"two words\"").unwrap().as_str(),
            Some("two words")
        );
        assert!(
            parse_value("interval", "[5, 9]")
                .unwrap()
                .as_array()
                .is_some()
        );
        assert!(
            parse_value("size", "50")
                .unwrap_err()
                .contains("multiple of 32")
        );
        assert!(
            parse_value("size", "big")
                .unwrap_err()
                .contains("multiple of 32")
        );
        assert!(parse_value("animation", "wiggle").is_err());
        assert!(
            parse_value("shiny", "yes")
                .unwrap_err()
                .contains("unknown key `shiny`")
        );
    }

    #[test]
    fn set_creates_the_host_table_and_keeps_everything_else() {
        let text = "# my fleet\nsize = 32 # small everywhere\n\n[hosts.\"prod-*\"]\naccent = \"cranberry\" # loud\n";
        let out = set(text, "OWL", "size", "128");
        assert!(
            out.starts_with("# my fleet\nsize = 32 # small everywhere\n"),
            "{out}"
        );
        assert!(out.contains("accent = \"cranberry\" # loud"), "{out}");
        let cfg = Config::parse(&out);
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        let s = cfg.effective("owl", &Layer::default());
        assert_eq!(
            (s.size.value, s.size.source),
            (128, Source::Host("owl".into()))
        );

        // Again: replaced in place, its comment kept.
        let edited = out.replace("size = 128", "size = 128 # tray");
        let again = set(&edited, "owl", "size", "64");
        assert!(again.contains("size = 64 # tray"), "{again}");
        assert_eq!(again.matches("[hosts.owl]").count(), 1, "{again}");
    }

    #[test]
    fn set_reuses_an_existing_table_in_any_case() {
        let out = set(
            "[hosts.BuildBox]\nmascot = \"owl\"\n",
            "buildbox",
            "animation",
            "static",
        );
        assert_eq!(
            out,
            "[hosts.BuildBox]\nmascot = \"owl\"\nanimation = \"static\"\n"
        );
        assert_eq!(
            set("", "owl", "mascot", "otter"),
            "[hosts.owl]\nmascot = \"otter\"\n"
        );
    }

    #[test]
    fn unset_removes_and_tidies() {
        let text = set("size = 32\n", "owl", "size", "128");
        let (out, removed) = unset_host_key(&text, "OWL", "size").unwrap();
        assert!(removed);
        assert_eq!(out, "size = 32\n", "empty tables go away");
        let (same, removed) = unset_host_key(&out, "owl", "size").unwrap();
        assert!(!removed);
        assert_eq!(same, out);

        let kept = "[hosts.owl]\nmascot = \"owl\"\nsize = 128\n";
        let (out, _) = unset_host_key(kept, "owl", "size").unwrap();
        assert_eq!(out, "[hosts.owl]\nmascot = \"owl\"\n");
    }

    #[test]
    fn broken_files_are_left_alone() {
        assert!(set_host_key("size = [", "owl", "size", Value::from(64)).is_err());
        assert!(set_host_key("hosts = 3", "owl", "size", Value::from(64)).is_err());
        assert!(unset_host_key("size = [", "owl", "size").is_err());
    }
}
