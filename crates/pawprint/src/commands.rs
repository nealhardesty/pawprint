//! Commands that work without a running badge or any window backend.

use std::path::Path;

use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::config::{Config, Layer, Settings};
use pawprint_core::host::LabelSetting;
use pawprint_core::identity::{self, Resolved};
use pawprint_core::render::{self, BadgeSpec};
use pawprint_core::sprite::CATEGORIES;
use pawprint_core::state::State;

use crate::paths::Paths;
use crate::{CmdResult, exit, hostname};

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
    let mut resolved = identity::resolve(
        &hostname,
        &config,
        &state,
        flags,
        Catalog::bundled(),
        &bundle().accents,
        recompute,
    );
    resolved.warnings.extend(warning);
    Ok((
        resolved,
        config_note,
        paths.state_file.display().to_string(),
    ))
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
