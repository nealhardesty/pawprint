//! Editing `config.toml` without disturbing it (SPEC §8.6): `pawprint set`,
//! `pawprint unset`, and the tray's Size and Animation choices all write one
//! key into this host's exact `[hosts."<short>"]` table with `toml_edit`, so
//! comments, order, and formatting survive. Values are checked by the same
//! parser that loads the file ([`config::check_value`]). The settings window
//! (SPEC §8.7) edits any table the same way ([`Scope`]).

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
label = "auto"             # "auto" (short hostname) | "full" | "none" | any text, shown as written
label_case = "upper"       # "upper" | "lower" | "as-is"
label_size = "auto"        # "auto" (tallest that fits) | "small" | "medium" | "large" (cap 7, 14, 21 px)

# Mascot size in logical pixels. 32, 64, and 128 are the presets; any multiple of 32 works.
size = 64

# Where the badge sits on the target display. Dragging in move mode overrides this.
position = "top-right"     # top-left | top | top-right | left | center | right | bottom-left | bottom | bottom-right
offset = [16, 16]          # logical px inward from the anchor (each -100000 to 100000)
display = "primary"        # "primary" | an output name from `pawprint doctor`

# Animation.
animation = "idle"         # "idle" | "static"
interval = [8, 20]         # seconds between idle actions (min, max), 1 <= min <= max <= 3600

# Crate color.
accent = "auto"            # "auto" | a palette name (see `pawprint list --accents`) | "#rrggbb"

# Tray icon (Windows), menu-bar item (macOS), or StatusNotifierItem (Linux).
tray = true

# When a fullscreen window covers the badge's display (a remote-desktop client,
# a video, a game): "hide" steps aside so the badge never shows over it.
fullscreen = "hide"        # "hide" | "show"

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

/// The `[hosts]` table's key for host `short`: the exact table loading
/// uses ([`config::is_exact_host`], the last match), else `short`
/// lowercased.
fn host_key(hosts: &Table, short: &str) -> String {
    hosts
        .iter()
        .map(|(k, _)| k)
        .filter(|k| config::is_exact_host(k, short))
        .last()
        .map_or_else(|| short.to_lowercase(), str::to_string)
}

fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|e| format!("config.toml is not valid TOML, so it was left alone: {e}"))
}

/// Which table an edit goes into.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The top level: every host, unless a host table says otherwise.
    Top,
    /// `[hosts."<key>"]`, with the key exactly as the file writes it.
    Host(String),
}

/// The `[hosts]` table, created (implicit, so no bare `[hosts]` header is
/// written) if missing.
fn hosts_mut(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    doc.entry("hosts")
        .or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        })
        .as_table_mut()
        .ok_or_else(|| "`hosts` in config.toml is not a table, so it was left alone".into())
}

/// The key of this host's exact table in `text`: the one loading uses, else
/// the short name lowercased (a table `set` would create).
pub fn this_host_key(text: &str, short: &str) -> String {
    text.parse::<DocumentMut>()
        .ok()
        .and_then(|doc| doc.get("hosts")?.as_table().map(|h| host_key(h, short)))
        .unwrap_or_else(|| short.to_lowercase())
}

/// Set `key = value` in `scope`, creating the host table (and `[hosts]`)
/// if needed. A value already there keeps its trailing comment.
pub fn set_in(text: &str, scope: &Scope, key: &str, value: Value) -> Result<String, String> {
    let mut doc = parse(text)?;
    let table = match scope {
        Scope::Top => doc.as_table_mut(),
        Scope::Host(name) => hosts_mut(&mut doc)?
            .entry(name)
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("[hosts.{name:?}] in config.toml is not a table"))?,
    };
    let decor = table
        .get(key)
        .and_then(Item::as_value)
        .map(|v| v.decor().clone());
    let mut value = value;
    if let Some(d) = decor {
        *value.decor_mut() = d;
    }
    table[key] = Item::Value(value);
    Ok(doc.to_string())
}

/// Remove `key` from `scope`. Returns the new text and whether the key was
/// there. With `tidy`, a host table left empty (and without comments) is
/// removed, and so is an empty implicit `[hosts]`.
pub fn unset_in(
    text: &str,
    scope: &Scope,
    key: &str,
    tidy: bool,
) -> Result<(String, bool), String> {
    let mut doc = parse(text)?;
    let name = match scope {
        Scope::Top => {
            let was = doc.remove(key).is_some();
            return Ok((if was { doc.to_string() } else { text.into() }, was));
        }
        Scope::Host(name) => name,
    };
    let Some(hosts) = doc.get_mut("hosts").and_then(Item::as_table_mut) else {
        return Ok((text.to_string(), false));
    };
    let Some(host) = hosts.get_mut(name).and_then(Item::as_table_mut) else {
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
    if tidy && host.is_empty() && !commented {
        hosts.remove(name);
    }
    if tidy && hosts.is_empty() && hosts.is_implicit() {
        doc.remove("hosts");
    }
    Ok((doc.to_string(), true))
}

/// What a `[hosts]` key may be: a hostname, an FQDN, or a glob (`*`, `?`).
pub fn check_host_key(key: &str) -> Result<(), String> {
    let ok = !key.is_empty()
        && key.len() <= 253
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '*' | '?'));
    if ok {
        Ok(())
    } else {
        Err("use letters, digits, - _ . and the wildcards * and ?".into())
    }
}

fn host_exists(hosts: &Table, key: &str) -> bool {
    hosts.iter().any(|(k, _)| k.eq_ignore_ascii_case(key))
}

/// Add an empty `[hosts."<key>"]` table at the end. Refuses a key already
/// there in any case, since loading would treat the two as one host.
pub fn add_host(text: &str, key: &str) -> Result<String, String> {
    check_host_key(key)?;
    let mut doc = parse(text)?;
    let hosts = hosts_mut(&mut doc)?;
    if host_exists(hosts, key) {
        return Err(format!("[hosts.{key:?}] is already there"));
    }
    hosts.insert(key, Item::Table(Table::new()));
    Ok(doc.to_string())
}

/// Rename `[hosts."<old>"]`, keeping its place, its keys, and its comments.
pub fn rename_host(text: &str, old: &str, new: &str) -> Result<String, String> {
    check_host_key(new)?;
    let mut doc = parse(text)?;
    let hosts = hosts_mut(&mut doc)?;
    if !old.eq_ignore_ascii_case(new) && host_exists(hosts, new) {
        return Err(format!("[hosts.{new:?}] is already there"));
    }
    let Some(item) = hosts.remove(old) else {
        return Err(format!("[hosts.{old:?}] is not in config.toml"));
    };
    hosts.insert(new, item);
    Ok(doc.to_string())
}

/// Remove `[hosts."<key>"]` and everything in it.
pub fn remove_host(text: &str, key: &str) -> Result<String, String> {
    let mut doc = parse(text)?;
    let hosts = hosts_mut(&mut doc)?;
    if hosts.remove(key).is_none() {
        return Err(format!("[hosts.{key:?}] is not in config.toml"));
    }
    if hosts.is_empty() && hosts.is_implicit() {
        doc.remove("hosts");
    }
    Ok(doc.to_string())
}

/// A loaded value (`Layer::get`) as a value to write, checked like loading.
pub fn to_edit(key: &str, value: &toml::Value) -> Result<Value, String> {
    config::check_value(key, value)?;
    value
        .to_string()
        .parse::<Value>()
        .map_err(|e| format!("{key}: {e}"))
}

/// Set `key = value` in `[hosts."<short>"]`, creating the table (and
/// `[hosts]`) if needed. A value already there keeps its trailing comment.
pub fn set_host_key(text: &str, short: &str, key: &str, value: Value) -> Result<String, String> {
    let name = {
        let mut doc = parse(text)?;
        host_key(hosts_mut(&mut doc)?, short)
    };
    set_in(text, &Scope::Host(name), key, value)
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
    parse(text)?;
    unset_in(text, &Scope::Host(this_host_key(text, short)), key, true)
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
    fn set_edits_the_table_loading_uses() {
        for text in [
            "[hosts.OWL]\nsize = 32\n[hosts.owl]\nsize = 32\n",
            "[hosts.owl]\nsize = 32\n[hosts.\"OWL.example.com\"]\nsize = 32\n",
        ] {
            let out = set(text, "owl", "size", "128");
            let s = Config::parse(&out).effective("owl", &Layer::default());
            assert_eq!(s.size.value, 128, "{out}");
            let (out, removed) = unset_host_key(&out, "owl", "size").unwrap();
            assert!(removed);
            let s = Config::parse(&out).effective("owl", &Layer::default());
            assert_eq!(s.size.value, 32, "{out}");
        }
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
    fn scoped_edits_reach_any_table() {
        let text = "# fleet\nsize = 32 # small\n\n[hosts.\"prod-*\"]\naccent = \"cranberry\"\n";
        let top = set_in(text, &Scope::Top, "size", Value::from(64)).unwrap();
        assert!(top.starts_with("# fleet\nsize = 64 # small\n"), "{top}");
        let glob = Scope::Host("prod-*".into());
        let out = set_in(&top, &glob, "label_case", Value::from("lower")).unwrap();
        let cfg = Config::parse(&out);
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        assert_eq!(
            cfg.effective("prod-web", &Layer::default())
                .label_case
                .source,
            Source::Host("prod-*".into())
        );
        // Without tidying, an emptied table stays (the settings window
        // removes tables only when asked).
        let (kept, was) = unset_in(&out, &glob, "accent", false).unwrap();
        let (kept, _) = unset_in(&kept, &glob, "label_case", false).unwrap();
        assert!(was);
        assert!(kept.contains("[hosts.\"prod-*\"]"), "{kept}");
        let (gone, _) = unset_in(&kept, &Scope::Top, "size", true).unwrap();
        assert!(!gone.contains("size"), "{gone}");
        let (same, was) = unset_in(&gone, &Scope::Top, "size", true).unwrap();
        assert!(!was);
        assert_eq!(same, gone);
    }

    #[test]
    fn host_tables_are_added_renamed_and_removed_in_place() {
        let text = "[hosts.a]\nsize = 32 # keep\n\n# about b\n[hosts.b]\nsize = 64\n";
        let added = add_host(text, "web-*").unwrap();
        assert!(added.ends_with("[hosts.\"web-*\"]\n"), "{added}");
        assert!(add_host(&added, "A").unwrap_err().contains("already"));
        assert!(add_host(&added, "bad key!").is_err());
        let renamed = rename_host(&added, "a", "alpha").unwrap();
        assert!(
            renamed.starts_with("[hosts.alpha]\nsize = 32 # keep\n"),
            "renamed in place: {renamed}"
        );
        assert!(rename_host(&renamed, "alpha", "b").is_err(), "taken");
        assert!(rename_host(&renamed, "zzz", "y").is_err(), "missing");
        assert_eq!(
            Config::parse(&renamed).host_keys,
            ["alpha", "b", "web-*"],
            "order kept"
        );
        let removed = remove_host(&renamed, "b").unwrap();
        assert!(!removed.contains("[hosts.b]"), "{removed}");
        let only = remove_host(&remove_host(&removed, "alpha").unwrap(), "web-*").unwrap();
        assert_eq!(only, "", "a table's own comment goes with it");
    }

    #[test]
    fn values_convert_and_are_checked() {
        assert_eq!(to_edit("size", &64.into()).unwrap().as_integer(), Some(64));
        let pair = toml::Value::Array(vec![1.into(), 2.into()]);
        assert!(to_edit("interval", &pair).unwrap().as_array().is_some());
        assert_eq!(
            to_edit("label", &"two \"words\"".into()).unwrap().as_str(),
            Some("two \"words\"")
        );
        assert!(to_edit("size", &50.into()).is_err());
        assert_eq!(this_host_key("[hosts.OWL]\n", "owl"), "OWL");
        assert_eq!(this_host_key("", "Owl"), "owl");
    }

    #[test]
    fn broken_files_are_left_alone() {
        assert!(set_host_key("size = [", "owl", "size", Value::from(64)).is_err());
        assert!(set_host_key("hosts = 3", "owl", "size", Value::from(64)).is_err());
        assert!(unset_host_key("size = [", "owl", "size").is_err());
    }
}
