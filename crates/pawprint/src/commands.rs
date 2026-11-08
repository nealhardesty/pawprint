//! Commands that work without a running badge or any window backend.

use std::path::Path;
use std::sync::OnceLock;

use pawprint_core::catalog::{Catalog, bundle, library};
use pawprint_core::config::{Config, Layer, Settings};
use pawprint_core::config_edit;
use pawprint_core::host::{self, LabelSetting};
use pawprint_core::identity::{self, Resolved, Rule};
use pawprint_core::render::{self, BadgeSpec};
use pawprint_core::sprite::{CATEGORIES, Origin, compile_dir, export_png};
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

/// Bundled plus user mascots, loaded once per process and shared by every
/// command (SPEC §12.7; the badge loads its own, again on each reload).
/// Invalid user mascots are left out and described in [`Catalog::warnings`].
pub fn catalog(paths: &Paths) -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| Catalog::load(&paths.user_mascots))
}

fn print_warnings(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
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
    let catalog = catalog(&paths);
    let mut resolved = resolve_with(&hostname, &config, &state, flags, catalog, recompute);
    resolved.warnings.extend(warning);
    resolved.warnings.extend_from_slice(catalog.warnings());
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
    catalog: &Catalog,
    recompute: bool,
) -> Resolved {
    identity::resolve(
        hostname,
        config,
        state,
        flags,
        catalog,
        &bundle().accents,
        recompute,
    )
}

pub fn explain(host: Option<String>, recompute: bool) -> CmdResult {
    let (r, config_note, state_note) = resolve(host, &Layer::default(), recompute)?;
    let s: &Settings = &r.settings;
    let row = |key: &str, value: &str, why: &str| println!("{key:<12}{value:<23}({why})");
    let label_how = match LabelSetting::parse(&s.label.value) {
        LabelSetting::Auto => "auto: short hostname, letters and digits",
        LabelSetting::Full => "full: full hostname, letters and digits",
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
    let catalog = catalog(&paths_or_err()?);
    print_warnings(catalog.warnings());
    println!(
        "{:<24}{:<20}{:<11}{:<10}ALIASES",
        "ID", "NAME", "CATEGORY", "SOURCE"
    );
    for (m, source) in catalog
        .entries()
        .filter(|(m, _)| category.as_ref().is_none_or(|c| *c == m.category))
    {
        println!(
            "{:<24}{:<20}{:<11}{:<10}{}",
            m.id,
            m.name,
            m.category,
            source,
            m.aliases.join(", ")
        );
    }
    Ok(())
}

pub fn render(id: &str, out: &Path, size: u32, label: Option<String>, frame: usize) -> CmdResult {
    let catalog = catalog(&paths_or_err()?);
    print_warnings(catalog.warnings());
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
        display_scale: 1.0,
    };
    let img = render::render_badge(&spec, bundle());
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
    write_config(&path, config_edit::TEMPLATE)?;
    println!("wrote {} (every key, at its default)", path.display());
    Ok(())
}

/// Write the config file through `edit`, which gets the current text (empty
/// if there is no file). A failed edit leaves the file alone and is a usage
/// error (exit 2); failing to read or write the file is exit 1.
pub fn edit_config(paths: &Paths, edit: impl FnOnce(&str) -> Result<String, String>) -> CmdResult {
    let path = &paths.config_file;
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err((exit::ERROR, format!("cannot read {}: {e}", path.display()))),
    };
    write_config(path, &edit(&text).map_err(|e| (exit::USAGE, e))?)
}

/// Replace the file atomically (temporary file, then rename), keeping its
/// permissions. A symlinked config (dotfile managers) is written at its
/// target, so the link stays; a link to nothing is an error, not replaced.
fn write_config(path: &Path, text: &str) -> CmdResult {
    let fail =
        |p: &Path, e: std::io::Error| (exit::ERROR, format!("cannot write {}: {e}", p.display()));
    let link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let target = match std::fs::canonicalize(path) {
        Ok(t) => t,
        Err(e) if link => {
            let to = std::fs::read_link(path).unwrap_or_default();
            return Err((
                exit::ERROR,
                format!(
                    "{} is a symlink to {}, which is missing ({e}); fix or remove the link",
                    path.display(),
                    to.display()
                ),
            ));
        }
        Err(_) => path.to_path_buf(),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(dir, e))?;
    }
    let tmp = target.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| fail(&tmp, e))?;
    if let Ok(meta) = std::fs::metadata(&target) {
        std::fs::set_permissions(&tmp, meta.permissions()).map_err(|e| fail(&tmp, e))?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| fail(&target, e))
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
    edit_config(&paths, |text| {
        let before = resolve_with(
            &hostname,
            &Config::parse(text),
            &state,
            &Layer::default(),
            catalog(&paths),
            false,
        );
        let after_text = config_edit::set(text, &short, key, raw)?;
        let after = resolve_with(
            &hostname,
            &Config::parse(&after_text),
            &state,
            &Layer::default(),
            catalog(&paths),
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
    })?;
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
    // A config that isn't valid TOML is the file's fault, not the command's.
    .map_err(|(_, e)| (exit::ERROR, e))?;
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

/// `pawprint mascot export <id> <dir>`: a bundled mascot in the simple PNG
/// form (SPEC §12.7), as a starting point for custom art.
pub fn mascot_export(id: &str, dir: &Path) -> CmdResult {
    let Some(m) = bundle().mascot(id) else {
        return Err((
            exit::ERROR,
            format!("no bundled mascot {id:?}; see `pawprint list`"),
        ));
    };
    let (toml_path, png_path) = (dir.join("mascot.toml"), dir.join("sprite.png"));
    if let Some(p) = [&toml_path, &png_path].into_iter().find(|p| p.exists()) {
        return Err((
            exit::ERROR,
            format!("{} already exists; export into a new folder", p.display()),
        ));
    }
    let failed = |path: &Path, e: std::io::Error| (exit::ERROR, format!("{}: {e}", path.display()));
    let (toml, strip) = export_png(m);
    std::fs::create_dir_all(dir).map_err(|e| failed(dir, e))?;
    std::fs::write(&toml_path, toml).map_err(|e| failed(&toml_path, e))?;
    std::fs::write(&png_path, strip.encode_png()).map_err(|e| failed(&png_path, e))?;
    println!(
        "wrote {} and {} ({} frames, {} actions)\n\n\
         Next: edit sprite.png, then run `pawprint mascot check {}`.\n\
         To use it, copy the folder to {}: as {id}/ it replaces the built-in;\n\
         with a new id (in mascot.toml and as the folder name) it adds a mascot.",
        toml_path.display(),
        png_path.display(),
        m.frames.len(),
        m.actions.len(),
        dir.display(),
        paths_or_err()?.user_mascots.display(),
    );
    Ok(())
}

/// `pawprint mascot check <dir>`: the build's validation rules for user
/// mascots (SPEC §12.11), plus alias collisions with the bundled catalog.
pub fn mascot_check(dir: &Path) -> CmdResult {
    // Canonical, so `.` still has a folder name to match the id against.
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let compiled = compile_dir(&dir, library(), Origin::User);
    let report = &compiled.report;
    report.errors.iter().for_each(|e| println!("error: {e}"));
    report
        .warnings
        .iter()
        .for_each(|w| println!("warning: {w}"));
    let Some(m) = compiled.mascot else {
        return Err((
            exit::ERROR,
            format!("{}: {} error(s)", dir.display(), report.errors.len()),
        ));
    };
    let role = match bundle().mascot(&m.id) {
        Some(_) => format!("replaces the bundled {}", m.id),
        None => "adds a new mascot".to_string(),
    };
    let summary = format!(
        "{} OK: {} frames, {} actions; it {role}",
        m.id,
        m.frames.len(),
        m.actions.len()
    );
    let with_bundled = Catalog::with_user(bundle().mascots.clone(), vec![m]);
    with_bundled
        .warnings()
        .iter()
        .for_each(|w| println!("warning: {w}"));
    println!("{summary}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn config_writes_keep_permissions_and_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = std::env::temp_dir().join(format!("pawprint-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        // A private config stays private.
        let file = dir.join("config.toml");
        std::fs::write(&file, "size = 32\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_config(&file, "size = 64\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "size = 64\n");
        assert_eq!(mode(&file), 0o600);

        // A link is written through, and stays a link.
        let link = dir.join("link.toml");
        symlink(&file, &link).unwrap();
        write_config(&link, "size = 128\n").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "size = 128\n");

        // A link to nothing is refused and left alone.
        let dangling = dir.join("dangling.toml");
        symlink(dir.join("gone.toml"), &dangling).unwrap();
        let (code, msg) = write_config(&dangling, "size = 32\n").unwrap_err();
        assert_eq!(code, exit::ERROR);
        assert!(
            msg.contains("symlink") && msg.contains("gone.toml"),
            "{msg}"
        );
        assert!(std::fs::symlink_metadata(&dangling).unwrap().is_symlink());
        assert!(!dir.join("gone.toml").exists());

        // An I/O failure is exit 1, not a usage error.
        let blocked = dir.join("file-not-dir").join("config.toml");
        std::fs::write(dir.join("file-not-dir"), "").unwrap();
        assert_eq!(write_config(&blocked, "").unwrap_err().0, exit::ERROR);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exported_mascots_pass_check_and_are_not_overwritten() {
        let root = std::env::temp_dir().join(format!("pawprint-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("walrus");
        assert_eq!(mascot_export("walrus", &dir), Ok(()));
        assert!(dir.join("mascot.toml").is_file() && dir.join("sprite.png").is_file());
        assert_eq!(mascot_check(&dir), Ok(()));
        assert!(mascot_export("walrus", &dir).is_err(), "never overwrites");
        assert!(mascot_export("no-such-mascot", &root.join("x")).is_err());

        let renamed = root.join("not-walrus");
        std::fs::rename(&dir, &renamed).unwrap();
        assert!(mascot_check(&renamed).is_err(), "id must match the folder");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
