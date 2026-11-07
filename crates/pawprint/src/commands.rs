//! Commands that work without a running badge or any window backend.

use std::path::Path;

use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::config::{Config, Layer, Settings};
use pawprint_core::config_edit;
use pawprint_core::host::{self, LabelSetting};
use pawprint_core::identity::{self, Resolved, Rule};
use pawprint_core::render::{self, BadgeSpec};
use pawprint_core::sprite::CATEGORIES;
use pawprint_core::state::State;

use crate::paths::Paths;
use crate::{CmdResult, exit, hostname, ipc};

fn paths_or_err() -> Result<Paths, (u8, String)> {
    Paths::detect().ok_or((exit::ERROR, "cannot find the home directory".into()))
}

/// Config file, or defaults if there is none. The note says which.
pub fn load_config(paths: &Paths) -> (Config, String) {
    match std::fs::read_to_string(&paths.config_file) {
        Ok(text) => (
            Config::parse(&text),
            paths.config_file.display().to_string(),
        ),
        Err(_) => (
            Config::default(),
            format!(
                "{} (not found; using defaults)",
                paths.config_file.display()
            ),
        ),
    }
}

/// Resolve this host (or `host`) the way the badge would, without writing
/// anything.
fn resolve(
    host: Option<String>,
    flags: &Layer,
    recompute: bool,
) -> Result<(Resolved, String, String), (u8, String)> {
    let paths = paths_or_err()?;
    let hostname = match host {
        Some(h) => h,
        None => hostname::detect().map_err(|e| (exit::ERROR, e))?,
    };
    let (config, config_note) = load_config(&paths);
    let (state, warning) = State::load(&paths.state_file);
    let mut resolved = resolve_with(&hostname, &config, &state, flags, recompute);
    resolved.warnings.extend(warning);
    Ok((
        resolved,
        config_note,
        paths.state_file.display().to_string(),
    ))
}

fn resolve_with(
    hostname: &str,
    config: &Config,
    state: &State,
    flags: &Layer,
    recompute: bool,
) -> Resolved {
    identity::resolve(
        hostname,
        config,
        state,
        flags,
        Catalog::bundled(),
        &bundle().accents,
        recompute,
    )
}

pub fn explain(host: Option<String>, recompute: bool) -> CmdResult {
    let (r, config_note, state_note) = resolve(host, &Layer::default(), recompute)?;
    let s: &Settings = &r.settings;
    let row = |key: &str, value: &str, why: &str| println!("{key:<12}{value:<23}({why})");
    let label_how = match LabelSetting::parse(&s.label.value) {
        LabelSetting::Auto => "auto: short hostname",
        LabelSetting::Full => "full: full hostname",
        LabelSetting::None => "none",
        LabelSetting::Text(_) => "literal text",
    };

    println!("{:<12}{}", "hostname", r.raw);
    println!("{:<12}{}", "short name", r.short);
    row(
        "label",
        r.label.as_deref().unwrap_or("(none)"),
        &format!(
            "{label_how}, {} case; {}",
            s.label_case.value, s.label.source
        ),
    );
    let won = r.trace.last().expect("a rule always wins");
    row(
        "mascot",
        &r.mascot,
        &format!("rule {}: {}", won.rule.number(), won.outcome),
    );
    row("accent", &r.accent.name, &r.accent.source.to_string());
    row(
        "size",
        &s.size.value.to_string(),
        &s.size.source.to_string(),
    );
    row(
        "label size",
        &s.label_size.value.to_string(),
        &s.label_size.source.to_string(),
    );
    row(
        "position",
        &r.position.value.to_string(),
        &r.position.source.to_string(),
    );
    row("display", &s.display.value, &s.display.source.to_string());
    row(
        "animation",
        &s.animation.value.to_string(),
        &s.animation.source.to_string(),
    );
    let [lo, hi] = s.interval.value;
    row(
        "interval",
        &format!("{lo}-{hi} s"),
        &s.interval.source.to_string(),
    );
    row(
        "tray",
        &s.tray.value.to_string(),
        &s.tray.source.to_string(),
    );
    row(
        "backend",
        &s.backend.value.to_string(),
        &s.backend.source.to_string(),
    );

    println!("\nmascot rules (first match wins)");
    for step in &r.trace {
        let mark = if std::ptr::eq(step, won) {
            "  <- chosen"
        } else {
            ""
        };
        println!(
            "  {} {:<24}{}{mark}",
            step.rule.number(),
            step.rule.title(),
            step.outcome
        );
    }
    println!("\nconfig      {config_note}");
    println!("state       {state_note}");
    for w in &r.warnings {
        eprintln!("warning: {w}");
    }
    Ok(())
}

pub fn list(category: Option<String>, accents: bool) -> CmdResult {
    if accents {
        for (i, (name, rgb)) in bundle().accents.iter().enumerate() {
            println!("{i:>2}  {name:<10} {}", rgb.to_hex());
        }
        return Ok(());
    }
    if let Some(c) = &category
        && !CATEGORIES.contains(&c.as_str())
    {
        return Err((
            exit::USAGE,
            format!(
                "unknown category {c:?}; choose one of {}",
                CATEGORIES.join(", ")
            ),
        ));
    }
    println!(
        "{:<24}{:<20}{:<11}{:<9}ALIASES",
        "ID", "NAME", "CATEGORY", "SOURCE"
    );
    for m in Catalog::bundled()
        .mascots()
        .iter()
        .filter(|m| category.as_ref().is_none_or(|c| *c == m.category))
    {
        println!(
            "{:<24}{:<20}{:<11}{:<9}{}",
            m.id,
            m.name,
            m.category,
            "bundled",
            m.aliases.join(", ")
        );
    }
    Ok(())
}

pub fn render(id: &str, out: &Path, size: u32, label: Option<String>, frame: usize) -> CmdResult {
    let catalog = Catalog::bundled();
    let Some(mascot) = catalog.get(id) else {
        return Err((
            exit::ERROR,
            format!("no mascot {id:?}; see `pawprint list`"),
        ));
    };
    if frame >= mascot.frames.len() {
        return Err((
            exit::ERROR,
            format!("{id} has frames 0-{}", mascot.frames.len() - 1),
        ));
    }
    // Render the badge this host would show with this mascot.
    let flags = Layer {
        mascot: Some(id.to_string()),
        size: Some(size),
        label: label.clone(),
        ..Layer::default()
    };
    let (r, _, _) = resolve(None, &flags, false)?;
    let spec = BadgeSpec {
        mascot,
        frame,
        label: r.label.as_deref(),
        accent: r.accent.rgb,
        size,
        label_size: r.settings.label_size.value,
        anchor: r.position.value.anchor,
        display_scale: 1.0,
    };
    let img = render::render_badge(&spec, &bundle().crate_art, &bundle().font);
    std::fs::write(out, img.encode_png())
        .map_err(|e| (exit::ERROR, format!("cannot write {}: {e}", out.display())))?;
    println!("wrote {} ({}x{})", out.display(), img.width, img.height);
    Ok(())
}

pub fn paths() -> CmdResult {
    let p = paths_or_err()?;
    let rows = [
        ("config", p.config_file.display().to_string()),
        ("user mascots", p.user_mascots.display().to_string()),
        ("state", p.state_file.display().to_string()),
        ("log", p.log_file.display().to_string()),
        ("ipc", p.ipc.clone()),
        ("binary", p.binary.display().to_string()),
    ];
    for (k, v) in rows {
        println!("{k:<14}{v}");
    }
    Ok(())
}

pub fn config_path() -> CmdResult {
    println!("{}", paths_or_err()?.config_file.display());
    Ok(())
}

/// `pawprint config init`: the commented template, never over a file.
pub fn config_init() -> CmdResult {
    let path = paths_or_err()?.config_file;
    if path.exists() {
        return Err((
            exit::ERROR,
            format!(
                "{} already exists; edit it, or delete it first for a fresh one",
                path.display()
            ),
        ));
    }
    write_config(&path, config_edit::TEMPLATE).map_err(|e| (exit::ERROR, e))?;
    println!("wrote {} (every key, at its default)", path.display());
    Ok(())
}

/// Write the config file through `edit`, which gets the current text (empty
/// if there is no file). A failed edit leaves the file alone.
pub fn edit_config(
    paths: &Paths,
    edit: impl FnOnce(&str) -> Result<String, String>,
) -> Result<(), String> {
    let path = &paths.config_file;
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    write_config(path, &edit(&text)?)
}

/// Replace the file atomically (temporary file, then rename). A symlinked
/// config (dotfile managers) is written at its target, so the link stays.
fn write_config(path: &Path, text: &str) -> Result<(), String> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", target.display());
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = target.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(fail)?;
    std::fs::rename(&tmp, &target).map_err(fail)
}

/// Tell this session's badge to re-read its config, if one is running.
/// Returns what to tell the user.
fn reload_running() -> &'static str {
    let Ok(endpoint) = crate::instance::endpoint() else {
        return "It applies when PAWPRINT starts.";
    };
    match ipc::call(&endpoint, ipc::Request::Reload) {
        Ok(r) if r.ok => "The running badge reloaded it.",
        _ => "It applies when PAWPRINT starts.",
    }
}

/// `pawprint set <key> <value>` (SPEC §8.6): into this host's exact
/// `[hosts."<short>"]` table, checked by the loader's own parser, then
/// resolved once more so a mascot or accent that doesn't exist is refused
/// before anything is written.
pub fn set(key: &str, raw: &str) -> CmdResult {
    let paths = paths_or_err()?;
    let hostname = hostname::detect().map_err(|e| (exit::ERROR, e))?;
    let short = host::short_name(&hostname);
    let (state, _) = State::load(&paths.state_file);
    let usage = |e: String| (exit::USAGE, e);
    edit_config(&paths, |text| {
        let before = resolve_with(
            &hostname,
            &Config::parse(text),
            &state,
            &Layer::default(),
            false,
        );
        let after_text = config_edit::set(text, &short, key, raw)?;
        let after = resolve_with(
            &hostname,
            &Config::parse(&after_text),
            &state,
            &Layer::default(),
            false,
        );
        let new: Vec<&String> = after
            .warnings
            .iter()
            .filter(|w| !before.warnings.contains(w))
            .collect();
        match new.as_slice() {
            [] => Ok(after_text),
            [w, ..] => Err(w.to_string()),
        }
    })
    .map_err(usage)?;
    println!(
        "Set {key} for {short} in [hosts.\"{short}\"] of {}. {}",
        paths.config_file.display(),
        reload_running()
    );
    Ok(())
}

/// `pawprint unset <key>`: remove it from this host's table again.
pub fn unset(key: &str) -> CmdResult {
    if !pawprint_core::config::KEYS.contains(&key) {
        return Err((
            exit::USAGE,
            format!(
                "unknown key `{key}`; the keys are {}",
                pawprint_core::config::KEYS.join(", ")
            ),
        ));
    }
    let paths = paths_or_err()?;
    let short = host::short_name(&hostname::detect().map_err(|e| (exit::ERROR, e))?);
    let mut removed = false;
    edit_config(&paths, |text| {
        let (out, was) = config_edit::unset_host_key(text, &short, key)?;
        removed = was;
        Ok(out)
    })
    .map_err(|e| (exit::ERROR, e))?;
    if removed {
        println!(
            "Removed {key} from [hosts.\"{short}\"]. {}",
            reload_running()
        );
    } else {
        println!(
            "{key} is not set in [hosts.\"{short}\"]; nothing to remove (`pawprint explain` shows where each setting comes from)."
        );
    }
    Ok(())
}

/// `pawprint forget`: drop the cached fallback mascot, in the running
/// badge if there is one (it saves the state), else in the state file.
pub fn forget() -> CmdResult {
    let (r, _, _) = resolve(None, &Layer::default(), false)?;
    if r.rule != Rule::Cached {
        println!(
            "Nothing cached for {}: its mascot comes from rule {} ({}).",
            r.short,
            r.rule.number(),
            r.rule.title()
        );
        return Ok(());
    }
    let running = crate::instance::endpoint()
        .ok()
        .and_then(|e| ipc::call(&e, ipc::Request::Forget).ok())
        .is_some_and(|r| r.ok);
    if !running {
        let paths = paths_or_err()?;
        let (mut state, _) = State::load(&paths.state_file);
        state.assignments.remove(&r.short);
        state.save_atomic(&paths.state_file).map_err(|e| {
            (
                exit::ERROR,
                format!("cannot write {}: {e}", paths.state_file.display()),
            )
        })?;
    }
    let (now, _, _) = resolve(None, &Layer::default(), true)?;
    println!(
        "Forgot {}'s cached mascot ({}); it gets {} now.",
        r.short, r.mascot, now.mascot
    );
    Ok(())
}
