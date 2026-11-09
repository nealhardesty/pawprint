//! The settings window (SPEC §8.7): every key in `config.toml`, for this
//! host, for every host, or for any `[hosts]` table, as a pure state
//! machine. The shell feeds it [`Input`] and executes its [`Effect`]s, the
//! way it drives the badge: the window is drawn here, once, in pixel art,
//! and backends only show the frames and report input.
//!
//! Every change is written at once (with `toml_edit`, so comments and
//! layout survive) and the running badge reloads, so the badge on the
//! desktop is the preview. Undo walks back through the session's writes.

use std::collections::HashMap;
use std::sync::Arc;

use pawprint_sprite::{Bundle, CATEGORIES, Rgb, RgbaImage};

use crate::accent;
use crate::catalog::{Catalog, Provenance};
use crate::config::{self, Config, Layer, Source};
use crate::config_edit::{self, Scope};
use crate::geom::{PhysicalRect, Point};
use crate::host::{self, LabelSetting};
use crate::identity;
use crate::layout::Anchor;
use crate::render::{self, BadgeSpec, Frame};
use crate::state::State;
use crate::surface::{DisplayInfo, select_display};
use crate::ui::{Canvas, Input, Rect, UiKey, text_width, theme};

/// The window in units (SPEC §8.7); the shell scales it by a whole number.
pub const W: i32 = 360;
pub const H: i32 = 252;

const NAV_W: i32 = 78;
const CX: i32 = NAV_W + 8;
const CW: i32 = W - CX - 8;
const BODY_Y: i32 = 36;
const STATUS_Y: i32 = H - 16;
const CHIP_H: i32 = 11;
const FIELD_H: i32 = 11;
/// Mascot grid cell: a 32-unit tile and a 1-unit frame on each side.
const CELL: i32 = 34;
const GRID_COLS: i32 = 7;
const HOST_ROW: i32 = 14;
const SCROLL_LINE: i32 = 12;
const UNDO_LIMIT: usize = 200;

/// What the shell does for the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Present(Frame),
    Place(PhysicalRect),
    /// Replace config.toml with this text (atomically), then tell the
    /// running badge to reload. On failure, call [`App::failed`].
    Write(String),
    /// Forget the dragged position (the running badge's, or the state
    /// file's), then call [`App::set_state`].
    ResetPosition,
    Close,
}

/// What the window is opened with.
#[derive(Debug, Clone)]
pub struct Inputs {
    pub hostname: String,
    /// config.toml's text, empty if there is none.
    pub text: String,
    pub state: State,
    /// Where config.toml is, to show.
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Mascot,
    Label,
    Look,
    Place,
    Motion,
    System,
    Hosts,
}

const PAGES: [(Page, &str); 7] = [
    (Page::Mascot, "MASCOT"),
    (Page::Label, "LABEL"),
    (Page::Look, "SIZE+COLOR"),
    (Page::Place, "POSITION"),
    (Page::Motion, "ANIMATION"),
    (Page::System, "SYSTEM"),
    (Page::Hosts, "HOST TABLES"),
];

/// Which part of the file the window edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Editing {
    /// This host's exact `[hosts."<short>"]` table, as `pawprint set` uses.
    ThisHost,
    /// The top level: every host.
    AllHosts,
    /// Another `[hosts]` table, by its key as written.
    Table(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FieldId {
    Label,
    Size,
    OffsetX,
    OffsetY,
    IntervalMin,
    IntervalMax,
    Accent,
    Display,
    NewHost,
    Rename,
}

/// What a click lands on.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    Page(Page),
    Editing(Editing),
    Set(&'static str, toml::Value),
    Clear(&'static str),
    Field(FieldId),
    Step(FieldId, i64),
    Category(Option<&'static str>),
    ScrollTo(i32),
    Undo,
    ForgetDrag,
    HostEdit(String),
    HostRename(String),
    HostDelete(String),
    HostAdd,
    RenameOk,
    Close,
}

/// A text field being typed into.
#[derive(Debug, Clone)]
struct Typing {
    id: FieldId,
    text: String,
    /// In characters.
    cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Good,
    Bad,
}

/// How a setting looks in the current scope.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    value: toml::Value,
    /// Set in the table being edited (else inherited).
    here: bool,
    /// Where an inherited value comes from.
    from: String,
    /// Set here, but this host gets something else from a stronger table.
    shadow: Option<String>,
}

pub struct App {
    catalog: Arc<Catalog>,
    assets: &'static Bundle,
    hostname: String,
    short: String,
    path: String,
    text: String,
    config: Config,
    /// Why the file can't be edited (not valid TOML).
    broken: Option<String>,
    state: State,
    editing: Editing,
    page: Page,
    displays: Vec<DisplayInfo>,
    /// Units to physical pixels.
    k: u32,
    pointer: Option<(i32, i32)>,
    hits: Vec<(Rect, Target)>,
    typing: Option<Typing>,
    category: Option<&'static str>,
    search: String,
    /// Scroll offsets in units.
    grid_top: i32,
    hosts_top: i32,
    rename: Option<String>,
    confirm_delete: Option<String>,
    status: Option<(String, Tone)>,
    undo: Vec<String>,
    thumbs: HashMap<(String, Rgb), RgbaImage>,
    presented: Option<RgbaImage>,
    placed: Option<PhysicalRect>,
    done: bool,
}

impl App {
    pub fn new(catalog: Arc<Catalog>, assets: &'static Bundle, inputs: Inputs) -> App {
        let short = host::short_name(&inputs.hostname);
        let mut app = App {
            catalog,
            assets,
            hostname: inputs.hostname,
            short,
            path: inputs.path,
            text: String::new(),
            config: Config::default(),
            broken: None,
            state: inputs.state,
            editing: Editing::ThisHost,
            page: Page::Mascot,
            displays: Vec::new(),
            k: 2,
            pointer: None,
            hits: Vec::new(),
            typing: None,
            category: None,
            search: String::new(),
            grid_top: 0,
            hosts_top: 0,
            rename: None,
            confirm_delete: None,
            status: None,
            undo: Vec::new(),
            thumbs: HashMap::new(),
            presented: None,
            placed: None,
            done: false,
        };
        app.set_text(inputs.text);
        app
    }

    /// Show `page` and present it, even if it is already showing (for
    /// `make settings-shots`).
    pub fn set_page(&mut self, page: Page) -> Vec<Effect> {
        self.page = page;
        self.presented = None;
        self.redraw_now()
    }

    /// Every page, in the order the window lists them.
    pub fn pages() -> impl Iterator<Item = Page> {
        PAGES.iter().map(|(p, _)| *p)
    }

    /// True once the window should close.
    pub fn done(&self) -> bool {
        self.done
    }

    /// The display the window opens on: the primary.
    pub fn target(&self) -> Option<&DisplayInfo> {
        select_display(&self.displays, "primary")
    }

    /// First call: size, place, and draw the window.
    pub fn start(&mut self, displays: Vec<DisplayInfo>) -> Vec<Effect> {
        self.displays_changed(displays)
    }

    /// Re-place (and redraw) when the display's scale or size changed.
    pub fn displays_changed(&mut self, displays: Vec<DisplayInfo>) -> Vec<Effect> {
        self.displays = displays;
        let mut out = Vec::new();
        if let Some(d) = self.target() {
            let work = d.work_area;
            let mut k = ((2.0 * d.scale).round() as u32).max(1);
            while k > 1 && (W as u32 * k > work.width || H as u32 * k > work.height) {
                k -= 1;
            }
            let (w, h) = (W as u32 * k, H as u32 * k);
            let rect = PhysicalRect::new(
                work.x + (work.width as i32 - w as i32) / 2,
                work.y + (work.height as i32 - h as i32) / 2,
                w,
                h,
            );
            if self.placed.is_none_or(|p| (p.width, p.height) != (w, h)) {
                self.k = k;
                self.placed = Some(rect);
                self.presented = None;
                out.push(Effect::Place(rect));
            }
        }
        self.redraw(&mut out);
        out
    }

    /// The file changed on disk, not through this window: show it.
    pub fn reload(&mut self, text: String) -> Vec<Effect> {
        if text == self.text {
            return Vec::new();
        }
        self.set_text(text);
        self.undo.clear();
        self.say(
            "config.toml changed on disk; showing the new version",
            Tone::Info,
        );
        self.redraw_now()
    }

    /// The last [`Effect::Write`] failed: go back to what is on disk.
    pub fn failed(&mut self, error: &str) -> Vec<Effect> {
        if let Some(previous) = self.undo.pop() {
            self.set_text(previous);
        }
        self.say(&format!("not saved: {error}"), Tone::Bad);
        self.redraw_now()
    }

    /// The state file changed (the dragged position was forgotten).
    pub fn set_state(&mut self, state: State) -> Vec<Effect> {
        self.state = state;
        self.redraw_now()
    }

    pub fn input(&mut self, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Move(p) => {
                // Only crossing from one control to another changes the
                // picture (hover, the preview, the mascot's name).
                let at = Some(self.units(p));
                let before = self.target_at(self.pointer).cloned();
                self.pointer = at;
                if self.target_at(at) == before.as_ref() {
                    return out;
                }
            }
            Input::Leave => self.pointer = None,
            Input::Press(p) => {
                self.pointer = Some(self.units(p));
                let target = self.target_at(self.pointer).cloned();
                // Clicking anywhere else finishes the field being typed in.
                if self.typing.is_some()
                    && !matches!(&target, Some(Target::Field(id)) if Some(*id) == self.typing.as_ref().map(|t| t.id))
                    && !matches!(target, Some(Target::RenameOk | Target::HostAdd))
                {
                    self.commit_field(&mut out);
                }
                if !matches!(target, Some(Target::HostDelete(_))) {
                    self.confirm_delete = None;
                }
                if let Some(t) = target {
                    self.activate(t, &mut out);
                }
            }
            Input::Release(_) => {}
            Input::Scroll(lines) => self.scroll((lines * f64::from(SCROLL_LINE)).round() as i32),
            Input::Key(key) => self.key(key, &mut out),
            Input::Text(c) => {
                if let Some(t) = &mut self.typing {
                    let at = byte_at(&t.text, t.cursor);
                    t.text.insert(at, c);
                    t.cursor += 1;
                } else if self.page == Page::Mascot && !c.is_whitespace() {
                    self.search.push(c);
                    self.grid_top = 0;
                }
            }
            Input::Shortcut('z') => self.undo(&mut out),
            Input::Shortcut('w' | 'q') | Input::Close => {
                self.commit_field(&mut out);
                self.close(&mut out);
            }
            Input::Shortcut(_) => {}
        }
        self.redraw(&mut out);
        out
    }

    // ----- state -----

    fn target_at(&self, at: Option<(i32, i32)>) -> Option<&Target> {
        let (x, y) = at?;
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| t)
    }

    fn set_text(&mut self, text: String) {
        self.broken = text
            .parse::<toml_edit::DocumentMut>()
            .err()
            .map(|e| e.to_string());
        self.config = Config::parse(&text);
        self.text = text;
        if let Editing::Table(key) = &self.editing
            && !self.config.host_keys.contains(key)
        {
            self.editing = Editing::ThisHost;
        }
    }

    fn units(&self, p: Point) -> (i32, i32) {
        let k = self.k.max(1) as i32;
        (p.x.div_euclid(k), p.y.div_euclid(k))
    }

    fn say(&mut self, text: &str, tone: Tone) {
        self.status = Some((text.to_string(), tone));
    }

    fn close(&mut self, out: &mut Vec<Effect>) {
        self.done = true;
        out.push(Effect::Close);
    }

    /// The table edits go into, as `config_edit` names it.
    fn scope(&self) -> Scope {
        match &self.editing {
            Editing::ThisHost => Scope::Host(config_edit::this_host_key(&self.text, &self.short)),
            Editing::AllHosts => Scope::Top,
            Editing::Table(key) => Scope::Host(key.clone()),
        }
    }

    fn scope_name(&self) -> String {
        match &self.editing {
            Editing::ThisHost => format!("this host ({})", self.short),
            Editing::AllHosts => "all hosts".into(),
            Editing::Table(key) => format!("[hosts.{key:?}]"),
        }
    }

    /// Index of a host table in `config.hosts` by its key as written.
    fn table_index(&self, key: &str) -> Option<usize> {
        self.config.host_keys.iter().rposition(|k| k == key)
    }

    /// This host's exact table, the one loading uses.
    fn this_host_index(&self) -> Option<usize> {
        self.config
            .host_keys
            .iter()
            .rposition(|k| config::is_exact_host(k, &self.short))
    }

    fn layer_here(&self) -> Option<&Layer> {
        match &self.editing {
            Editing::AllHosts => Some(&self.config.top),
            Editing::ThisHost => self.this_host_index().map(|i| &self.config.hosts[i].1),
            Editing::Table(key) => self.table_index(key).map(|i| &self.config.hosts[i].1),
        }
    }

    /// Does the table being edited apply to this host?
    fn applies_here(&self) -> bool {
        match &self.editing {
            Editing::ThisHost | Editing::AllHosts => true,
            Editing::Table(key) => table_matches(key, &self.short),
        }
    }

    fn effective(&self, config: &Config) -> config::Settings {
        config
            .effective_checked(
                &self.short,
                &Layer::default(),
                |id| self.catalog.get(id).is_some(),
                |name| accent::lookup(name, &self.assets.accents).is_some(),
            )
            .0
    }

    /// The value of `key` in the current scope, set there or inherited.
    fn shown(&self, key: &'static str) -> Shown {
        let here = self.layer_here().and_then(|l| l.get(key));
        // What applies below the table being edited.
        let below = match &self.editing {
            Editing::ThisHost => {
                let mut c = self.config.clone();
                c.hosts
                    .retain(|(p, _)| !config::is_exact_host(p, &self.short));
                self.effective(&c)
            }
            Editing::AllHosts => self.effective(&Config::default()),
            Editing::Table(_) => self.effective(&Config {
                top: self.config.top.clone(),
                ..Config::default()
            }),
        };
        let below = below.get(key).expect("a known key");
        let shadow = here.as_ref().and_then(|v| {
            if !self.applies_here() {
                return None;
            }
            let real = self.effective(&self.config).get(key).expect("a known key");
            let mine = match &self.editing {
                Editing::AllHosts => Source::TopLevel,
                Editing::ThisHost => {
                    Source::Host(self.config.hosts[self.this_host_index()?].0.clone())
                }
                Editing::Table(k) => {
                    Source::Host(self.config.hosts[self.table_index(k)?].0.clone())
                }
            };
            (real.source != mine && real.value != *v).then(|| {
                format!(
                    "{} uses {} {}",
                    self.short,
                    show_value(&real.value),
                    source_text(&real.source)
                )
            })
        });
        match here {
            Some(value) => Shown {
                value,
                here: true,
                from: String::new(),
                shadow,
            },
            None => Shown {
                value: below.value,
                here: false,
                from: source_text(&below.source),
                shadow: None,
            },
        }
    }

    fn shown_str(&self, key: &'static str) -> String {
        match self.shown(key).value {
            toml::Value::String(s) => s,
            v => v.to_string(),
        }
    }

    fn shown_pair(&self, key: &'static str) -> [i64; 2] {
        match self.shown(key).value.as_array().map(Vec::as_slice) {
            Some([a, b]) => [a.as_integer().unwrap_or(0), b.as_integer().unwrap_or(0)],
            _ => [0, 0],
        }
    }

    // ----- edits -----

    fn edit(
        &mut self,
        out: &mut Vec<Effect>,
        what: &str,
        change: impl FnOnce(&str) -> Result<String, String>,
    ) -> bool {
        if let Some(e) = &self.broken {
            let e = first_line(e);
            self.say(
                &format!("config.toml is not valid TOML ({e}); fix it in a text editor"),
                Tone::Bad,
            );
            return false;
        }
        match change(&self.text) {
            Ok(text) if text == self.text => true,
            Ok(text) => {
                self.undo.push(std::mem::take(&mut self.text));
                if self.undo.len() > UNDO_LIMIT {
                    self.undo.remove(0);
                }
                self.set_text(text.clone());
                self.say(what, Tone::Good);
                out.push(Effect::Write(text));
                true
            }
            Err(e) => {
                self.say(&e, Tone::Bad);
                false
            }
        }
    }

    fn set(&mut self, key: &'static str, value: toml::Value, out: &mut Vec<Effect>) -> bool {
        let edit = match config_edit::to_edit(key, &value) {
            Ok(v) => v,
            Err(e) => {
                self.say(&tidy_warning(&e), Tone::Bad);
                return false;
            }
        };
        let ok = match key {
            "mascot" => value
                .as_str()
                .is_some_and(|m| m == "auto" || self.catalog.get(m).is_some()),
            "accent" => value
                .as_str()
                .is_some_and(|a| a == "auto" || accent::lookup(a, &self.assets.accents).is_some()),
            _ => true,
        };
        if !ok {
            self.say(
                &format!("{} is not available here", show_value(&value)),
                Tone::Bad,
            );
            return false;
        }
        let scope = self.scope();
        let what = format!("{key} = {} for {}", show_value(&value), self.scope_name());
        self.edit(out, &what, |text| {
            config_edit::set_in(text, &scope, key, edit)
        })
    }

    fn clear(&mut self, key: &'static str, out: &mut Vec<Effect>) {
        let scope = self.scope();
        // This host's table goes away when empty, as with `pawprint unset`;
        // other tables stay until removed on purpose.
        let tidy = self.editing == Editing::ThisHost;
        let what = format!("{key} cleared for {}", self.scope_name());
        self.edit(out, &what, |text| {
            config_edit::unset_in(text, &scope, key, tidy).map(|(t, _)| t)
        });
    }

    fn undo(&mut self, out: &mut Vec<Effect>) {
        self.typing = None;
        match self.undo.pop() {
            Some(text) => {
                self.set_text(text.clone());
                self.say("undone", Tone::Info);
                out.push(Effect::Write(text));
            }
            None => self.say("nothing to undo", Tone::Info),
        }
    }

    fn activate(&mut self, target: Target, out: &mut Vec<Effect>) {
        match target {
            Target::Page(p) => {
                self.page = p;
                self.rename = None;
            }
            Target::Editing(e) => self.editing = e,
            Target::Set(key, value) => {
                if key == "label" && value.as_str() == Some("") {
                    // TEXT: type the label.
                    self.focus(FieldId::Label);
                } else {
                    self.set(key, value, out);
                }
            }
            Target::Clear(key) => self.clear(key, out),
            Target::Field(id) => {
                if self.typing.as_ref().map(|t| t.id) != Some(id) {
                    self.focus(id);
                }
                if let Some((x, _)) = self.pointer
                    && let Some((r, _)) = self
                        .hits
                        .iter()
                        .rev()
                        .find(|(_, t)| *t == Target::Field(id))
                    && let Some(t) = &mut self.typing
                {
                    let n = t.text.chars().count();
                    t.cursor = (((x - r.x - 2) + 3) / 6).clamp(0, n as i32) as usize;
                }
            }
            Target::Step(id, delta) => self.step(id, delta, out),
            Target::Category(c) => {
                self.category = c;
                self.grid_top = 0;
            }
            Target::ScrollTo(top) => match self.page {
                Page::Hosts => self.hosts_top = top.max(0),
                _ => self.grid_top = top.max(0),
            },
            Target::Undo => self.undo(out),
            Target::ForgetDrag => out.push(Effect::ResetPosition),
            Target::HostEdit(key) => {
                self.editing = if config::is_exact_host(&key, &self.short)
                    && self.this_host_index().map(|i| &self.config.host_keys[i]) == Some(&key)
                {
                    Editing::ThisHost
                } else {
                    Editing::Table(key)
                };
                self.page = Page::Mascot;
            }
            Target::HostRename(key) => {
                self.rename = Some(key);
                self.focus(FieldId::Rename);
            }
            Target::RenameOk => self.commit_field(out),
            Target::HostDelete(key) => {
                if self.confirm_delete.as_deref() == Some(key.as_str()) {
                    self.confirm_delete = None;
                    if self.edit(out, &format!("removed [hosts.{key:?}]"), |t| {
                        config_edit::remove_host(t, &key)
                    }) && self.editing == Editing::Table(key)
                    {
                        self.editing = Editing::ThisHost;
                    }
                } else {
                    self.confirm_delete = Some(key);
                }
            }
            Target::HostAdd => {
                if self
                    .typing
                    .as_ref()
                    .is_some_and(|t| t.id == FieldId::NewHost)
                {
                    self.commit_field(out);
                } else {
                    self.focus(FieldId::NewHost);
                }
            }
            Target::Close => self.close(out),
        }
    }

    fn focus(&mut self, id: FieldId) {
        let text = match id {
            FieldId::Label => match LabelSetting::parse(&self.shown_str("label")) {
                LabelSetting::Text(t) => t,
                _ => String::new(),
            },
            FieldId::Size => self.shown_str("size"),
            FieldId::OffsetX => self.shown_pair("offset")[0].to_string(),
            FieldId::OffsetY => self.shown_pair("offset")[1].to_string(),
            FieldId::IntervalMin => self.shown_pair("interval")[0].to_string(),
            FieldId::IntervalMax => self.shown_pair("interval")[1].to_string(),
            // The fields hold what the chips beside them don't offer.
            FieldId::Accent => Some(self.shown_str("accent"))
                .filter(|a| a.starts_with('#'))
                .unwrap_or_default(),
            FieldId::Display => Some(self.shown_str("display"))
                .filter(|d| d != "primary" && !self.displays.iter().any(|x| x.name == *d))
                .unwrap_or_default(),
            FieldId::NewHost => String::new(),
            FieldId::Rename => self.rename.clone().unwrap_or_default(),
        };
        let cursor = text.chars().count();
        self.typing = Some(Typing { id, text, cursor });
    }

    /// Finish the field being typed in: write its value if it changed.
    fn commit_field(&mut self, out: &mut Vec<Effect>) {
        let Some(Typing { id, text, .. }) = self.typing.take() else {
            return;
        };
        let text = text.trim().to_string();
        let int = || {
            text.parse::<i64>()
                .map_err(|_| format!("{text:?} is not a whole number"))
        };
        let result: Result<(), String> = (|| {
            match id {
                FieldId::Label => {
                    if text.is_empty() {
                        return Err("an empty label: choose NONE to hide it".into());
                    }
                    if matches!(text.as_str(), "auto" | "full" | "none") {
                        return Err(format!("{text:?} is a choice of its own: click it above"));
                    }
                    self.set_if_changed("label", text.clone().into(), out);
                }
                FieldId::Size => {
                    self.set_if_changed("size", int()?.into(), out);
                }
                FieldId::OffsetX | FieldId::OffsetY => {
                    let mut pair = self.shown_pair("offset");
                    pair[usize::from(id == FieldId::OffsetY)] = int()?;
                    self.set_if_changed("offset", pair_value(pair), out);
                }
                FieldId::IntervalMin | FieldId::IntervalMax => {
                    let pair = self.interval_with(id, int()?);
                    self.set_if_changed("interval", pair_value(pair), out);
                }
                FieldId::Accent => {
                    let a = text.to_lowercase();
                    if accent::lookup(&a, &self.assets.accents).is_none() && a != "auto" {
                        return Err(format!("{text:?}: use #rrggbb or a name from the swatches"));
                    }
                    self.set_if_changed("accent", a.into(), out);
                }
                FieldId::Display => {
                    if text.is_empty() {
                        return Err("an empty display name: choose PRIMARY instead".into());
                    }
                    self.set_if_changed("display", text.clone().into(), out);
                }
                FieldId::NewHost => {
                    if text.is_empty() {
                        return Ok(());
                    }
                    if self.edit(out, &format!("added [hosts.{text:?}]"), |t| {
                        config_edit::add_host(t, &text)
                    }) {
                        self.editing = Editing::Table(text.clone());
                        self.page = Page::Mascot;
                    }
                }
                FieldId::Rename => {
                    let Some(old) = self.rename.take() else {
                        return Ok(());
                    };
                    if text == old || text.is_empty() {
                        return Ok(());
                    }
                    let was_editing = self.editing == Editing::Table(old.clone());
                    if self.edit(out, &format!("renamed [hosts.{old:?}] to {text:?}"), |t| {
                        config_edit::rename_host(t, &old, &text)
                    }) && was_editing
                    {
                        self.editing = Editing::Table(text.clone());
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.say(&e, Tone::Bad);
        }
    }

    fn set_if_changed(&mut self, key: &'static str, value: toml::Value, out: &mut Vec<Effect>) {
        let shown = self.shown(key);
        if !(shown.here && shown.value == value) {
            self.set(key, value, out);
        }
    }

    /// The interval with one end changed, the other moved along if needed
    /// so that min <= max.
    fn interval_with(&self, id: FieldId, n: i64) -> [i64; 2] {
        let [lo, hi] = self.shown_pair("interval");
        let n = n.clamp(1, 3600);
        if id == FieldId::IntervalMin {
            [n, hi.max(n)]
        } else {
            [lo.min(n), n]
        }
    }

    fn step(&mut self, id: FieldId, delta: i64, out: &mut Vec<Effect>) {
        self.typing = None;
        match id {
            FieldId::Size => {
                let n = self.shown("size").value.as_integer().unwrap_or(64);
                let n = ((n + delta) / 32 * 32).clamp(32, 1024);
                self.set("size", n.into(), out);
            }
            FieldId::OffsetX | FieldId::OffsetY => {
                let mut pair = self.shown_pair("offset");
                let i = usize::from(id == FieldId::OffsetY);
                pair[i] = (pair[i] + delta).clamp(-config::MAX_OFFSET, config::MAX_OFFSET);
                self.set("offset", pair_value(pair), out);
            }
            FieldId::IntervalMin | FieldId::IntervalMax => {
                let [lo, hi] = self.shown_pair("interval");
                let now = if id == FieldId::IntervalMin { lo } else { hi };
                let pair = self.interval_with(id, now + delta);
                self.set("interval", pair_value(pair), out);
            }
            _ => {}
        }
    }

    fn key(&mut self, key: UiKey, out: &mut Vec<Effect>) {
        if let Some(t) = &mut self.typing {
            let n = t.text.chars().count();
            match key {
                UiKey::Left => t.cursor = t.cursor.saturating_sub(1),
                UiKey::Right => t.cursor = (t.cursor + 1).min(n),
                UiKey::Home | UiKey::Up => t.cursor = 0,
                UiKey::End | UiKey::Down => t.cursor = n,
                UiKey::Backspace if t.cursor > 0 => {
                    t.cursor -= 1;
                    let at = byte_at(&t.text, t.cursor);
                    t.text.remove(at);
                }
                UiKey::Delete if t.cursor < n => {
                    let at = byte_at(&t.text, t.cursor);
                    t.text.remove(at);
                }
                UiKey::Enter | UiKey::Tab => self.commit_field(out),
                UiKey::Escape => {
                    self.typing = None;
                    self.rename = None;
                }
                _ => {}
            }
            return;
        }
        match key {
            UiKey::Escape if !self.search.is_empty() => self.search.clear(),
            UiKey::Escape => self.close(out),
            UiKey::Backspace if self.page == Page::Mascot => {
                self.search.pop();
            }
            // Type to search, Enter to pick the first match.
            UiKey::Enter if self.page == Page::Mascot && !self.search.is_empty() => {
                if let Some(id) = self.matches().first().cloned() {
                    self.set("mascot", id.into(), out);
                    self.search.clear();
                }
            }
            UiKey::Up => self.scroll(-SCROLL_LINE),
            UiKey::Down => self.scroll(SCROLL_LINE),
            UiKey::PageUp => self.scroll(-8 * SCROLL_LINE),
            UiKey::PageDown => self.scroll(8 * SCROLL_LINE),
            UiKey::Home => self.scroll(-1_000_000),
            UiKey::End => self.scroll(1_000_000),
            UiKey::Tab => {
                let i = PAGES.iter().position(|(p, _)| *p == self.page).unwrap_or(0);
                self.page = PAGES[(i + 1) % PAGES.len()].0;
            }
            _ => {}
        }
    }

    fn scroll(&mut self, by: i32) {
        match self.page {
            Page::Mascot => self.grid_top = (self.grid_top + by).clamp(0, self.grid_max()),
            Page::Hosts => self.hosts_top = (self.hosts_top + by).clamp(0, self.hosts_max()),
            _ => {}
        }
    }

    // ----- drawing -----

    fn redraw_now(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        self.redraw(&mut out);
        out
    }

    fn redraw(&mut self, out: &mut Vec<Effect>) {
        if self.placed.is_none() || self.done {
            return;
        }
        let img = self.draw();
        if self.presented.as_ref() != Some(&img) {
            out.push(Effect::Present(Frame::from_rgba(&crate::ui::upscale(
                &img, self.k,
            ))));
            self.presented = Some(img);
        }
    }

    fn draw(&mut self) -> RgbaImage {
        let mut ui = Ui {
            c: Canvas::new(W, H, theme::BG),
            hits: Vec::new(),
            pointer: self.pointer,
            assets: self.assets,
        };
        self.draw_title(&mut ui);
        self.draw_scopes(&mut ui);
        self.draw_nav(&mut ui);
        self.draw_preview(&mut ui);
        ui.c.fill(
            Rect::new(NAV_W + 2, BODY_Y, W - NAV_W - 4, STATUS_Y - BODY_Y - 2),
            theme::PANEL,
        );
        if let Some(e) = self.broken.clone() {
            self.draw_broken(&mut ui, &e);
        } else {
            match self.page {
                Page::Mascot => self.draw_mascot(&mut ui),
                Page::Label => self.draw_label(&mut ui),
                Page::Look => self.draw_look(&mut ui),
                Page::Place => self.draw_place(&mut ui),
                Page::Motion => self.draw_motion(&mut ui),
                Page::System => self.draw_system(&mut ui),
                Page::Hosts => self.draw_hosts(&mut ui),
            }
        }
        self.draw_status(&mut ui);
        self.hits = ui.hits;
        ui.c.img
    }

    fn draw_title(&self, ui: &mut Ui) {
        ui.c.fill(Rect::new(0, 0, W, 15), theme::PANEL);
        ui.c.text(&self.assets.font, "PAWPRINT SETTINGS", 8, 4, theme::TEXT);
        let micro = &self.assets.micro_font;
        let room = (W - 130) / 4;
        let path = ellipsize_left(&self.path, room as usize);
        let w = text_width(micro, &path);
        ui.c.text(micro, &path, W - 8 - w, 5, theme::DIM);
    }

    fn draw_scopes(&self, ui: &mut Ui) {
        let y = 19;
        let micro = &self.assets.micro_font;
        ui.c.text(micro, "CHANGES APPLY TO", 8, y + 3, theme::DIM);
        let mut x = 8 + text_width(micro, "CHANGES APPLY TO") + 6;
        let mut chips = vec![
            (Editing::ThisHost, format!("THIS HOST: {}", self.short)),
            (Editing::AllHosts, "ALL HOSTS".to_string()),
        ];
        if let Editing::Table(k) = &self.editing {
            chips.push((self.editing.clone(), format!("[HOSTS.{k}]")));
        }
        for (e, label) in chips {
            let on = self.editing == e;
            x += ui.chip(x, y, &label, Pick::of(on, on), Target::Editing(e)) + 4;
        }
        if !self.applies_here() {
            ui.c.text(
                micro,
                &format!("(NOT {})", self.short.to_uppercase()),
                x,
                y + 3,
                theme::BAD,
            );
        } else if !matches!(self.editing, Editing::Table(_)) {
            ui.small(x, y + 1, "MORE...", Target::Page(Page::Hosts));
        }
    }

    fn draw_nav(&self, ui: &mut Ui) {
        for (i, (page, label)) in PAGES.iter().enumerate() {
            let r = Rect::new(4, BODY_Y + i as i32 * 14, NAV_W - 4, 13);
            let on = self.page == *page;
            let hover = ui.hovered(r);
            let bg = if on {
                theme::PICK
            } else if hover {
                theme::HOVER
            } else {
                theme::BG
            };
            ui.c.fill(r, bg);
            let fg = if on { theme::ON_PICK } else { theme::TEXT };
            ui.c.text(&self.assets.font, label, r.x + 4, r.y + 3, fg);
            ui.hit(r, Target::Page(*page));
        }
    }

    /// The badge as this host would show it with the file as it is, or as
    /// it would be with the hovered choice.
    fn draw_preview(&mut self, ui: &mut Ui) {
        let hovered = self.target_at(ui.pointer).and_then(|t| match t {
            Target::Set(key, value) if !(*key == "label" && value.as_str() == Some("")) => {
                Some((*key, value.clone()))
            }
            _ => None,
        });
        let config = match &hovered {
            Some((key, value)) if self.broken.is_none() => config_edit::to_edit(key, value)
                .and_then(|v| config_edit::set_in(&self.text, &self.scope(), key, v))
                .map(|t| Config::parse(&t))
                .unwrap_or_else(|_| self.config.clone()),
            _ => self.config.clone(),
        };
        let r = identity::resolve(
            &self.hostname,
            &config,
            &self.state,
            &Layer::default(),
            &self.catalog,
            &self.assets.accents,
            false,
        );
        let mascot = self.catalog.get(&r.mascot).expect("resolved mascots exist");
        let spec = BadgeSpec {
            mascot,
            frame: 0,
            label: r.label.as_deref(),
            accent: r.accent.rgb,
            size: 64,
            label_size: r.settings.label_size.value,
            display_scale: 1.0,
        };
        let img = render::render_badge(&spec, self.assets);
        let y = BODY_Y + PAGES.len() as i32 * 14 + 8;
        let panel = Rect::new(4, y, NAV_W - 4, STATUS_Y - y - 2);
        ui.c.fill(panel, theme::PANEL);
        let micro = &self.assets.micro_font;
        ui.c.text(
            micro,
            if hovered.is_some() {
                "PREVIEW"
            } else {
                "THIS HOST"
            },
            panel.x + 4,
            panel.y + 3,
            theme::DIM,
        );
        ui.c.blit(&img, panel.x + (panel.w - 64) / 2, panel.y + 11, 1);
        let mut line = panel.y + 78;
        for text in [
            r.mascot.clone(),
            format!("{} PX, {}", r.settings.size.value, r.accent.name),
        ] {
            let t = ellipsize_right(&text, ((panel.w - 6) / 4) as usize);
            ui.c.text(micro, &t, panel.x + 4, line, theme::DIM);
            line += 7;
        }
    }

    fn draw_status(&self, ui: &mut Ui) {
        let y = STATUS_Y;
        ui.c.fill(Rect::new(0, y, W, H - y), theme::PANEL);
        let (text, tone) = match &self.status {
            Some((t, tone)) => (t.clone(), *tone),
            None if self.typing.is_some() => ("Enter keeps it, Esc cancels".into(), Tone::Info),
            None if !self.config.warnings.is_empty() => (
                format!(
                    "{} warning(s) in config.toml: `pawprint explain` lists them",
                    self.config.warnings.len()
                ),
                Tone::Bad,
            ),
            None => (
                "Every change is saved at once; the badge follows".into(),
                Tone::Info,
            ),
        };
        let color = match tone {
            Tone::Info => theme::DIM,
            Tone::Good => theme::GOOD,
            Tone::Bad => theme::BAD,
        };
        let mut x = W - 8;
        x -= ui.measure_chip("CLOSE");
        ui.chip(x, y + 3, "CLOSE", Pick::Off, Target::Close);
        if !self.undo.is_empty() {
            x -= ui.measure_chip("UNDO") + 4;
            ui.chip(x, y + 3, "UNDO", Pick::Off, Target::Undo);
        }
        let room = ((x - 14) / 6).max(0) as usize;
        ui.c.text(
            &self.assets.font,
            &ellipsize_right(&text, room),
            8,
            y + 5,
            color,
        );
    }

    fn draw_broken(&self, ui: &mut Ui, error: &str) {
        let font = &self.assets.font;
        ui.c.text(
            font,
            "config.toml is not valid TOML,",
            CX,
            BODY_Y + 6,
            theme::BAD,
        );
        ui.c.text(
            font,
            "so it is not changed from here.",
            CX,
            BODY_Y + 16,
            theme::BAD,
        );
        ui.c.text(
            font,
            "Fix it in a text editor; this",
            CX,
            BODY_Y + 32,
            theme::TEXT,
        );
        ui.c.text(
            font,
            "window shows it once it loads.",
            CX,
            BODY_Y + 42,
            theme::TEXT,
        );
        let mut y = BODY_Y + 60;
        for line in error.lines().take(8) {
            let t = ellipsize_right(line, (CW / 4) as usize);
            ui.c.text(&self.assets.micro_font, &t, CX, y, theme::DIM);
            y += 8;
        }
    }

    /// A setting's title, where its value comes from, and CLEAR when it is
    /// set in this table. Returns the y for its control.
    fn header(&self, ui: &mut Ui, y: i32, key: &'static str, title: &str) -> i32 {
        let font = &self.assets.font;
        let micro = &self.assets.micro_font;
        ui.c.text(font, title, CX, y, theme::TEXT);
        let shown = self.shown(key);
        let mut x = CX + CW;
        if shown.here {
            x -= ui.measure_small("CLEAR");
            ui.small(x, y - 1, "CLEAR", Target::Clear(key));
            let t = "SET HERE";
            x -= text_width(micro, t) + 4;
            ui.c.text(micro, t, x, y + 1, theme::PICK);
        } else {
            let t = shown.from.to_uppercase();
            let t = ellipsize_right(&t, ((CW - text_width(font, title) - 8) / 4) as usize);
            x -= text_width(micro, &t);
            ui.c.text(micro, &t, x, y + 1, theme::DIM);
        }
        y + 10
    }

    /// The micro-font line under a control: a shadow warning, else `help`.
    fn note(&self, ui: &mut Ui, y: i32, key: &'static str, help: &str) -> i32 {
        let micro = &self.assets.micro_font;
        let (text, color) = match self.shown(key).shadow {
            Some(s) => (s, theme::BAD),
            None => (help.to_string(), theme::DIM),
        };
        let t = ellipsize_right(&text.to_uppercase(), (CW / 4) as usize);
        ui.c.text(micro, &t, CX, y, color);
        y + 12
    }

    /// One chip per choice; the shown value is picked.
    fn choices(
        &self,
        ui: &mut Ui,
        x: i32,
        y: i32,
        key: &'static str,
        choices: &[(&str, &str)],
    ) -> i32 {
        let shown = self.shown(key);
        let mut x = x;
        for (label, value) in choices {
            let v: toml::Value = (*value).into();
            let pick = Pick::of(shown.value == v, shown.here);
            x += ui.chip(x, y, label, pick, Target::Set(key, v)) + 3;
        }
        x
    }

    /// A text field; it shows `value` until it is clicked.
    fn field(&self, ui: &mut Ui, r: Rect, id: FieldId, value: &str, placeholder: &str) {
        let typing = self.typing.as_ref().filter(|t| t.id == id);
        let hover = ui.hovered(r);
        ui.c.fill(r, theme::FIELD);
        ui.c.border(
            r,
            if typing.is_some() {
                theme::PICK
            } else if hover {
                theme::DIM
            } else {
                theme::LINE
            },
        );
        let font = &self.assets.font;
        let inner = r.inset(1);
        ui.c.clip(inner);
        match typing {
            Some(t) => {
                let caret = t.cursor as i32 * 6;
                let shift = (caret - (inner.w - 4)).max(0);
                let x = inner.x + 2 - shift;
                ui.c.text(font, &t.text, x, inner.y + 1, theme::TEXT);
                ui.c.fill(Rect::new(x + caret - 1, inner.y, 1, inner.h), theme::PICK);
            }
            None if value.is_empty() => {
                ui.c.text(
                    &self.assets.micro_font,
                    placeholder,
                    inner.x + 2,
                    inner.y + 2,
                    theme::DIM,
                );
            }
            None => {
                ui.c.text(font, value, inner.x + 2, inner.y + 1, theme::TEXT);
            }
        }
        ui.c.unclip();
        ui.hit(r, Target::Field(id));
    }

    /// [-] field [+]: sizes step by 32, everything else by 1.
    fn stepper(&self, ui: &mut Ui, x: i32, y: i32, id: FieldId, value: i64, w: i32) -> i32 {
        let step = if id == FieldId::Size { 32 } else { 1 };
        let mut x = x;
        x += ui.chip(x, y, "-", Pick::Off, Target::Step(id, -step)) + 2;
        self.field(ui, Rect::new(x, y, w, FIELD_H), id, &value.to_string(), "");
        x += w + 2;
        x += ui.chip(x, y, "+", Pick::Off, Target::Step(id, step));
        x
    }

    fn draw_mascot(&mut self, ui: &mut Ui) {
        let mut y = BODY_Y + 4;
        y = self.header(ui, y, "mascot", "MASCOT");
        let shown = self.shown("mascot");
        let current = shown.value.as_str().unwrap_or("auto").to_string();
        let auto: toml::Value = "auto".into();
        let x = ui.chip(
            CX,
            y,
            "AUTO",
            Pick::of(current == "auto", shown.here),
            Target::Set("mascot", auto),
        );
        // What the pointer is on, else what is chosen.
        let hovered = self.target_at(ui.pointer).and_then(|t| match t {
            Target::Set("mascot", v) => v.as_str().map(str::to_string),
            _ => None,
        });
        let font = &self.assets.font;
        let info = match hovered
            .as_deref()
            .or((current != "auto").then_some(current.as_str()))
        {
            Some("auto") | None => "picked from the hostname".to_string(),
            Some(id) => match self.catalog.get(id) {
                Some(m) => format!("{} ({})", m.name, m.id),
                None => id.to_string(),
            },
        };
        let info = if self.search.is_empty() {
            info
        } else {
            format!("search: {}_", self.search)
        };
        ui.c.text(
            font,
            &ellipsize_right(&info, ((CW - x - 6) / 6) as usize),
            CX + x + 6,
            y + 2,
            theme::TEXT,
        );
        y += CHIP_H + 3;

        // Categories, in the micro font to fit on one line.
        let user = self
            .catalog
            .entries()
            .any(|(_, p)| p != Provenance::Bundled);
        let mut cats: Vec<(Option<&'static str>, String)> = vec![(None, "ALL".into())];
        cats.extend(CATEGORIES.iter().map(|c| (Some(*c), c.to_uppercase())));
        if user {
            cats.push((Some("yours"), "YOURS".into()));
        }
        let mut x = CX;
        for (c, label) in cats {
            let on = self.category == c;
            x += ui.small_pick(x, y, &label, on, Target::Category(c)) + 2;
        }
        y += 11;

        // The grid.
        let accent = self.preview_accent();
        let ids: Vec<(String, Rgb)> = self.matches().into_iter().map(|id| (id, accent)).collect();
        let area = Rect::new(CX, y, GRID_COLS * CELL, STATUS_Y - 4 - y);
        let rows = (ids.len() as i32 + GRID_COLS - 1) / GRID_COLS;
        let total = rows * CELL;
        self.grid_top = self.grid_top.clamp(0, (total - area.h).max(0));
        ui.c.clip(area);
        for (i, (id, accent)) in ids.iter().enumerate() {
            let (col, row) = (i as i32 % GRID_COLS, i as i32 / GRID_COLS);
            let cell = Rect::new(
                area.x + col * CELL,
                area.y + row * CELL - self.grid_top,
                CELL,
                CELL,
            );
            if cell.bottom() < area.y || cell.y > area.bottom() {
                continue;
            }
            let picked = current == *id;
            let hover = ui.hovered(cell)
                && area.contains(
                    ui.pointer.unwrap_or_default().0,
                    ui.pointer.unwrap_or_default().1,
                );
            ui.c.fill(
                cell.inset(1),
                if hover { theme::HOVER } else { theme::RAISED },
            );
            if picked {
                ui.c.border(cell, if shown.here { theme::PICK } else { theme::DIM });
            }
            let thumb = self.thumb(id, *accent);
            ui.c.blit(&thumb, cell.x + 1, cell.y + 1, 1);
            let visible = Rect::new(
                cell.x,
                cell.y.max(area.y),
                cell.w,
                cell.bottom().min(area.bottom()) - cell.y.max(area.y),
            );
            ui.hit(visible, Target::Set("mascot", id.as_str().into()));
        }
        ui.c.unclip();
        if ids.is_empty() {
            ui.c.text(
                font,
                "no mascot matches",
                area.x + 4,
                area.y + 4,
                theme::DIM,
            );
        }
        self.scrollbar(
            ui,
            Rect::new(area.right() + 4, area.y, 5, area.h),
            self.grid_top,
            total,
            area.h,
        );
    }

    /// The mascots the grid shows: this category, matching the search.
    fn matches(&self) -> Vec<String> {
        let q = self.search.to_lowercase();
        self.catalog
            .entries()
            .filter(|(m, p)| match self.category {
                None => true,
                Some("yours") => *p != Provenance::Bundled,
                Some(c) => m.category == c,
            })
            .filter(|(m, _)| {
                q.is_empty()
                    || m.id.contains(&q)
                    || m.name.to_lowercase().contains(&q)
                    || m.aliases.iter().any(|a| a.contains(&q))
            })
            .map(|(m, _)| m.id.clone())
            .collect()
    }

    fn grid_max(&self) -> i32 {
        let n = self.catalog.mascots().len() as i32;
        ((n + GRID_COLS - 1) / GRID_COLS * CELL - 100).max(0)
    }

    fn hosts_max(&self) -> i32 {
        (self.config.host_keys.len() as i32 * HOST_ROW).max(0)
    }

    /// A track the thumb moves in; clicking the track jumps there.
    fn scrollbar(&self, ui: &mut Ui, track: Rect, top: i32, total: i32, view: i32) {
        if total <= view {
            return;
        }
        ui.c.fill(track, theme::BG);
        let h = (track.h * view / total).max(6);
        let y = track.y + (track.h - h) * top / (total - view).max(1);
        ui.c.fill(Rect::new(track.x, y, track.w, h), theme::DIM);
        // Each slice of the track scrolls to its share of the list.
        let slices = 12;
        for i in 0..slices {
            let r = Rect::new(
                track.x - 1,
                track.y + track.h * i / slices,
                track.w + 2,
                track.h / slices + 1,
            );
            ui.hit(r, Target::ScrollTo((total - view) * i / (slices - 1)));
        }
    }

    /// The accent the thumbnails' crates use: this host's.
    fn preview_accent(&self) -> Rgb {
        identity::resolve(
            &self.hostname,
            &self.config,
            &self.state,
            &Layer::default(),
            &self.catalog,
            &self.assets.accents,
            false,
        )
        .accent
        .rgb
    }

    fn thumb(&mut self, id: &str, accent: Rgb) -> RgbaImage {
        let key = (id.to_string(), accent);
        if let Some(t) = self.thumbs.get(&key) {
            return t.clone();
        }
        let m = self.catalog.get(id).expect("listed mascots exist");
        let t = render::compose_1x(&self.assets.crate_art, m, 0, accent);
        self.thumbs.insert(key, t.clone());
        t
    }

    fn draw_label(&self, ui: &mut Ui) {
        let mut y = BODY_Y + 4;
        y = self.header(ui, y, "label", "LABEL ON THE CRATE");
        let shown = self.shown_str("label");
        let custom = matches!(LabelSetting::parse(&shown), LabelSetting::Text(_));
        let x = self.choices(
            ui,
            CX,
            y,
            "label",
            &[("AUTO", "auto"), ("FULL", "full"), ("NONE", "none")],
        );
        let here = self.shown("label").here;
        let x =
            x + ui.chip(
                x,
                y,
                "TEXT:",
                Pick::of(
                    custom || self.typing.as_ref().is_some_and(|t| t.id == FieldId::Label),
                    here,
                ),
                Target::Set("label", "".into()),
            ) + 3;
        let value = if custom { shown.clone() } else { String::new() };
        self.field(
            ui,
            Rect::new(x, y, CX + CW - x, FIELD_H),
            FieldId::Label,
            &value,
            "TYPE ANY TEXT",
        );
        y += CHIP_H + 3;
        y = self.note(
            ui,
            y,
            "label",
            "AUTO: the short hostname. FULL: the whole hostname.",
        );

        y = self.header(ui, y, "label_case", "LETTER CASE");
        self.choices(
            ui,
            CX,
            y,
            "label_case",
            &[
                ("UPPER", "upper"),
                ("lower", "lower"),
                ("As written", "as-is"),
            ],
        );
        y += CHIP_H + 3;
        y = self.note(ui, y, "label_case", "how the label's letters are shown");

        y = self.header(ui, y, "label_size", "TEXT SIZE");
        self.choices(
            ui,
            CX,
            y,
            "label_size",
            &[
                ("AUTO", "auto"),
                ("SMALL", "small"),
                ("MEDIUM", "medium"),
                ("LARGE", "large"),
            ],
        );
        y += CHIP_H + 3;
        self.note(
            ui,
            y,
            "label_size",
            "AUTO: the tallest text that fits on the crate",
        );
    }

    fn draw_look(&self, ui: &mut Ui) {
        let mut y = BODY_Y + 4;
        y = self.header(ui, y, "size", "MASCOT SIZE");
        let shown = self.shown("size");
        let mut x = CX;
        for n in crate::menu::SIZE_PRESETS {
            let v: toml::Value = i64::from(n).into();
            x += ui.chip(
                x,
                y,
                &n.to_string(),
                Pick::of(shown.value == v, shown.here),
                Target::Set("size", v),
            ) + 3;
        }
        x += 6;
        let n = shown.value.as_integer().unwrap_or(64);
        self.stepper(ui, x, y, FieldId::Size, n, 30);
        y += CHIP_H + 3;
        y = self.note(
            ui,
            y,
            "size",
            "logical pixels: any multiple of 32 up to 1024",
        );

        y = self.header(ui, y, "accent", "CRATE COLOR");
        let accent = self.shown_str("accent");
        let shown = self.shown("accent");
        let mut x = CX;
        x += ui.chip(
            x,
            y,
            "AUTO",
            Pick::of(accent == "auto", shown.here),
            Target::Set("accent", "auto".into()),
        ) + 6;
        let typed = if accent.starts_with('#') {
            accent.clone()
        } else {
            String::new()
        };
        self.field(
            ui,
            Rect::new(x, y, 64, FIELD_H),
            FieldId::Accent,
            &typed,
            "HEX COLOR",
        );
        let swatch_y = y + CHIP_H + 4;
        let mut hovered_name = None;
        for (i, (name, rgb)) in self.assets.accents.iter().enumerate() {
            let r = Rect::new(CX + i as i32 * 16, swatch_y, 14, 14);
            let on = accent == *name;
            if on {
                ui.c.fill(r, if shown.here { theme::PICK } else { theme::DIM });
            }
            ui.c.fill(r.inset(if on { 2 } else { 1 }), *rgb);
            if ui.hovered(r) {
                ui.c.border(r, theme::TEXT);
                hovered_name = Some(name.clone());
            }
            ui.hit(r, Target::Set("accent", name.as_str().into()));
        }
        y = swatch_y + 17;
        let help = match hovered_name {
            Some(n) => n,
            None => "AUTO: the mascot's own color".into(),
        };
        self.note(ui, y, "accent", &help);
    }

    fn draw_place(&self, ui: &mut Ui) {
        let mut y = BODY_Y + 4;
        y = self.header(ui, y, "position", "CORNER OR EDGE");
        let shown = self.shown("position");
        for (i, anchor) in Anchor::ALL.iter().enumerate() {
            let (col, row) = (i as i32 % 3, i as i32 / 3);
            let r = Rect::new(CX + col * 18, y + row * 12, 16, 10);
            let v: toml::Value = anchor.name().into();
            let on = shown.value == v;
            let hover = ui.hovered(r);
            let bg = match (on, shown.here) {
                (true, true) => theme::PICK,
                (true, false) => theme::DIM,
                _ if hover => theme::HOVER,
                _ => theme::RAISED,
            };
            ui.c.fill(r, bg);
            ui.hit(r, Target::Set("position", v));
        }
        let font = &self.assets.font;
        let micro = &self.assets.micro_font;
        let tx = CX + 62;
        ui.c.text(font, &self.shown_str("position"), tx, y + 2, theme::TEXT);
        match &self.state.position {
            Some(p) => {
                ui.c.text(
                    micro,
                    &format!(
                        "DRAGGED TO {} {:?}: THAT WINS",
                        p.anchor.name().to_uppercase(),
                        p.offset
                    ),
                    tx,
                    y + 13,
                    theme::BAD,
                );
                ui.small(tx, y + 21, "FORGET THE DRAG", Target::ForgetDrag);
            }
            None => {
                ui.c.text(micro, "OR DRAG IT: PAWPRINT MOVE,", tx, y + 13, theme::DIM);
                ui.c.text(micro, "OR MOVE IN THE TRAY MENU", tx, y + 20, theme::DIM);
            }
        }
        y += 38;
        if let Some(s) = self.shown("position").shadow {
            ui.c.text(
                micro,
                &ellipsize_right(&s.to_uppercase(), (CW / 4) as usize),
                CX,
                y - 3,
                theme::BAD,
            );
        }

        y = self.header(ui, y + 2, "offset", "DISTANCE FROM THE EDGE");
        let [ox, oy] = self.shown_pair("offset");
        let mut x = CX;
        x += ui.c.text(font, "X", x, y + 2, theme::DIM) + 4;
        x = self.stepper(ui, x, y, FieldId::OffsetX, ox, 36) + 12;
        x += ui.c.text(font, "Y", x, y + 2, theme::DIM) + 4;
        self.stepper(ui, x, y, FieldId::OffsetY, oy, 36);
        y += CHIP_H + 3;
        y = self.note(
            ui,
            y,
            "offset",
            "logical pixels inward from the corner or edge",
        );

        y = self.header(ui, y, "display", "DISPLAY");
        let shown = self.shown("display");
        let current = self.shown_str("display");
        let mut x = CX;
        let mut names: Vec<String> = vec!["primary".into()];
        names.extend(self.displays.iter().map(|d| d.name.clone()));
        for name in names {
            let label = if name == "primary" {
                "PRIMARY".to_string()
            } else {
                name.clone()
            };
            let w = ui.measure_chip(&label);
            if x + w > CX + CW - 70 {
                break;
            }
            let pick = Pick::of(current == name, shown.here);
            x += ui.chip(
                x,
                y,
                &label,
                pick,
                Target::Set("display", name.as_str().into()),
            ) + 3;
        }
        let known = current == "primary" || self.displays.iter().any(|d| d.name == current);
        let typed = if known { String::new() } else { current };
        self.field(
            ui,
            Rect::new(CX + CW - 66, y, 66, FIELD_H),
            FieldId::Display,
            &typed,
            "OTHER NAME",
        );
        y += CHIP_H + 3;
        self.note(ui, y, "display", "names as `pawprint doctor` lists them");
    }

    fn draw_motion(&self, ui: &mut Ui) {
        let mut y = BODY_Y + 4;
        y = self.header(ui, y, "animation", "ANIMATION");
        self.choices(
            ui,
            CX,
            y,
            "animation",
            &[("IDLE", "idle"), ("STATIC", "static")],
        );
        y += CHIP_H + 3;
        y = self.note(
            ui,
            y,
            "animation",
            "IDLE: a short action now and then. STATIC: never moves.",
        );

        y = self.header(ui, y, "interval", "SECONDS BETWEEN ACTIONS");
        let [lo, hi] = self.shown_pair("interval");
        let font = &self.assets.font;
        let mut x = CX;
        x += ui.c.text(font, "FROM", x, y + 2, theme::DIM) + 4;
        x = self.stepper(ui, x, y, FieldId::IntervalMin, lo, 30) + 12;
        x += ui.c.text(font, "TO", x, y + 2, theme::DIM) + 4;
        self.stepper(ui, x, y, FieldId::IntervalMax, hi, 30);
        y += CHIP_H + 3;
        self.note(ui, y, "interval", "a random wait in this range, 1 to 3600");
    }

    fn draw_system(&self, ui: &mut Ui) {
        let mut y = BODY_Y + 4;
        y = self.header(ui, y, "tray", "TRAY ICON");
        let shown = self.shown("tray");
        let mut x = CX;
        for (label, v) in [("SHOW", true), ("HIDE", false)] {
            let value: toml::Value = v.into();
            x += ui.chip(
                x,
                y,
                label,
                Pick::of(shown.value == value, shown.here),
                Target::Set("tray", value),
            ) + 3;
        }
        y += CHIP_H + 3;
        y = self.note(
            ui,
            y,
            "tray",
            "the icon in the notification area or menu bar",
        );

        y = self.header(ui, y, "fullscreen", "FULLSCREEN WINDOWS");
        self.choices(
            ui,
            CX,
            y,
            "fullscreen",
            &[("HIDE THE BADGE", "hide"), ("STAY ON TOP", "show")],
        );
        y += CHIP_H + 3;
        y = self.note(
            ui,
            y,
            "fullscreen",
            "HIDE steps aside for remote desktops, videos, games",
        );

        y = self.header(ui, y, "backend", "WINDOW BACKEND");
        self.choices(
            ui,
            CX,
            y,
            "backend",
            &[
                ("AUTO", "auto"),
                ("WINDOWS", "windows"),
                ("MACOS", "macos"),
                ("X11", "x11"),
                ("WAYLAND", "wayland"),
            ],
        );
        y += CHIP_H + 3;
        self.note(
            ui,
            y,
            "backend",
            "leave on AUTO unless `pawprint doctor` says otherwise",
        );
    }

    fn draw_hosts(&mut self, ui: &mut Ui) {
        let font = &self.assets.font;
        let micro = &self.assets.micro_font;
        let mut y = BODY_Y + 4;
        ui.c.text(font, "HOST TABLES", CX, y, theme::TEXT);
        y += 10;
        for line in [
            "A TABLE OVERRIDES ALL HOSTS ON THE MACHINES IT NAMES: A SHORT",
            "HOSTNAME, OR A GLOB WITH * AND ?. EXACT NAMES BEAT GLOBS.",
        ] {
            ui.c.text(micro, line, CX, y, theme::DIM);
            y += 7;
        }
        let legend = format!(
            "GREEN ONES APPLY TO THIS HOST ({}).",
            self.short.to_uppercase()
        );
        ui.c.text(micro, &legend, CX, y, theme::GOOD);
        y += 7;
        y += 3;
        let area = Rect::new(CX, y, CW - 8, STATUS_Y - 22 - y);
        let keys = self.config.host_keys.clone();
        let total = keys.len() as i32 * HOST_ROW;
        self.hosts_top = self.hosts_top.clamp(0, (total - area.h).max(0));
        ui.c.clip(area);
        for (i, key) in keys.iter().enumerate() {
            let row = Rect::new(
                area.x,
                area.y + i as i32 * HOST_ROW - self.hosts_top,
                area.w,
                HOST_ROW - 2,
            );
            if row.bottom() < area.y || row.y > area.bottom() {
                continue;
            }
            let editing = match &self.editing {
                Editing::Table(k) => k == key,
                Editing::ThisHost => {
                    self.this_host_index().map(|i| &self.config.host_keys[i]) == Some(key)
                }
                Editing::AllHosts => false,
            };
            ui.c.fill(row, if editing { theme::RAISED } else { theme::BG });
            let mut x = row.x + 3;
            if self.rename.as_deref() == Some(key.as_str()) {
                self.field(
                    ui,
                    Rect::new(x, row.y + 1, 120, FIELD_H - 1),
                    FieldId::Rename,
                    key,
                    "",
                );
                ui.small(x + 124, row.y + 2, "OK", Target::RenameOk);
                continue;
            }
            // This host's tables in green (the legend is above the list).
            let color = if table_matches(key, &self.short) {
                theme::GOOD
            } else {
                theme::TEXT
            };
            ui.c.text(font, &ellipsize_right(key, 14), x, row.y + 3, color);
            x = row.x + 92;
            let layer = &self.config.hosts[i].1;
            let n = config::KEYS
                .iter()
                .filter(|k| layer.get(k).is_some())
                .count();
            let kind = if key.contains(['*', '?']) {
                "GLOB"
            } else {
                "HOST"
            };
            ui.c.text(micro, &format!("{kind}, {n} SET"), x, row.y + 4, theme::DIM);
            let mut bx = row.right() - 2;
            let delete = if self.confirm_delete.as_deref() == Some(key.as_str()) {
                "REALLY?"
            } else {
                "DELETE"
            };
            for (label, target) in [
                (delete, Target::HostDelete(key.clone())),
                ("RENAME", Target::HostRename(key.clone())),
                ("EDIT", Target::HostEdit(key.clone())),
            ] {
                bx -= ui.measure_small(label);
                ui.small(bx, row.y + 2, label, target);
                bx -= 3;
            }
        }
        ui.c.unclip();
        if keys.is_empty() {
            ui.c.text(font, "none yet", area.x + 3, area.y + 3, theme::DIM);
        }
        self.scrollbar(
            ui,
            Rect::new(area.right() + 3, area.y, 5, area.h),
            self.hosts_top,
            total,
            area.h,
        );
        let y = STATUS_Y - 18;
        ui.c.text(font, "ADD", CX, y + 2, theme::TEXT);
        self.field(
            ui,
            Rect::new(CX + 24, y, 150, FIELD_H),
            FieldId::NewHost,
            "",
            "HOSTNAME OR GLOB, E.G. WEB-*",
        );
        ui.chip(CX + 178, y, "ADD TABLE", Pick::Off, Target::HostAdd);
    }
}

/// Is `[hosts."<key>"]` one that applies to host `short`?
fn table_matches(key: &str, short: &str) -> bool {
    let p = key.split('.').next().unwrap_or(key).to_lowercase();
    config::glob_match(&p, &short.to_lowercase())
}

fn pair_value([a, b]: [i64; 2]) -> toml::Value {
    toml::Value::Array(vec![a.into(), b.into()])
}

fn show_value(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn source_text(s: &Source) -> String {
    match s {
        Source::Default => "default".into(),
        Source::TopLevel => "from all hosts".into(),
        Source::Host(p) => format!("from [hosts.{p}]"),
        Source::Flag => "from a flag".into(),
        Source::StateFile => "dragged".into(),
    }
}

/// A loader warning, shortened for the status line.
fn tidy_warning(w: &str) -> String {
    w.trim_end_matches("; using the next layer down")
        .to_string()
}

fn first_line(s: &str) -> &str {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim()
}

fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

fn ellipsize_right(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep: String = s.chars().take(max.saturating_sub(3)).collect();
    format!("{keep}...")
}

fn ellipsize_left(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep: String = s.chars().skip(n - max.saturating_sub(3)).collect();
    format!("...{keep}")
}

/// How a chip looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Off,
    /// The shown value, set in this table.
    Here,
    /// The shown value, inherited.
    Inherited,
}

impl Pick {
    fn of(on: bool, here: bool) -> Pick {
        match (on, here) {
            (false, _) => Pick::Off,
            (true, true) => Pick::Here,
            (true, false) => Pick::Inherited,
        }
    }
}

/// One frame's canvas and the click targets drawn on it.
struct Ui {
    c: Canvas,
    hits: Vec<(Rect, Target)>,
    pointer: Option<(i32, i32)>,
    assets: &'static Bundle,
}

impl Ui {
    fn hit(&mut self, r: Rect, t: Target) {
        self.hits.push((r, t));
    }

    fn hovered(&self, r: Rect) -> bool {
        self.pointer.is_some_and(|(x, y)| r.contains(x, y))
    }

    fn measure_chip(&self, label: &str) -> i32 {
        text_width(&self.assets.font, label) + 8
    }

    fn measure_small(&self, label: &str) -> i32 {
        text_width(&self.assets.micro_font, label) + 6
    }

    /// A button in the regular font; returns its width.
    fn chip(&mut self, x: i32, y: i32, label: &str, pick: Pick, t: Target) -> i32 {
        let w = self.measure_chip(label);
        let r = Rect::new(x, y, w, CHIP_H);
        let hover = self.hovered(r);
        let (bg, fg) = match pick {
            Pick::Here => (theme::PICK, theme::ON_PICK),
            Pick::Inherited => (
                if hover { theme::HOVER } else { theme::RAISED },
                theme::PICK,
            ),
            Pick::Off => (
                if hover { theme::HOVER } else { theme::RAISED },
                theme::TEXT,
            ),
        };
        self.c.fill(r, bg);
        if pick == Pick::Inherited {
            self.c.border(r, theme::PICK);
        }
        self.c.text(&self.assets.font, label, x + 4, y + 2, fg);
        self.hit(r, t);
        w
    }

    /// A small button in the micro font; returns its width.
    fn small(&mut self, x: i32, y: i32, label: &str, t: Target) -> i32 {
        self.small_pick(x, y, label, false, t)
    }

    fn small_pick(&mut self, x: i32, y: i32, label: &str, on: bool, t: Target) -> i32 {
        let w = self.measure_small(label);
        let r = Rect::new(x, y, w, 9);
        let hover = self.hovered(r);
        let (bg, fg) = if on {
            (theme::PICK, theme::ON_PICK)
        } else if hover {
            (theme::HOVER, theme::TEXT)
        } else {
            (theme::RAISED, theme::TEXT)
        };
        self.c.fill(r, bg);
        self.c
            .text(&self.assets.micro_font, label, x + 3, y + 2, fg);
        self.hit(r, t);
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::bundle;
    use crate::geom::PhysicalRect;

    fn display() -> DisplayInfo {
        let r = PhysicalRect::new(0, 0, 1920, 1080);
        DisplayInfo {
            name: "DP-1".into(),
            bounds: r,
            work_area: PhysicalRect::new(0, 0, 1920, 1040),
            scale: 1.0,
            primary: true,
        }
    }

    struct Rig {
        app: App,
        written: Vec<String>,
        closed: bool,
    }

    impl Rig {
        fn new(text: &str) -> Rig {
            let inputs = Inputs {
                hostname: "owl.example.com".into(),
                text: text.into(),
                state: State::default(),
                path: "/home/me/.config/pawprint/config.toml".into(),
            };
            let mut app = App::new(Arc::new(Catalog::bundled().clone()), bundle(), inputs);
            let fx = app.start(vec![display()]);
            assert!(matches!(fx[0], Effect::Place(r) if r.width == W as u32 * 2));
            assert!(matches!(fx[1], Effect::Present(_)));
            Rig {
                app,
                written: Vec::new(),
                closed: false,
            }
        }

        fn run(&mut self, input: Input) {
            for e in self.app.input(input) {
                match e {
                    Effect::Write(t) => self.written.push(t),
                    Effect::Close => self.closed = true,
                    _ => {}
                }
            }
        }

        fn text(&self) -> &str {
            &self.app.text
        }

        /// Click the first target drawn that `want` accepts.
        fn click(&mut self, want: impl Fn(&Target) -> bool) {
            let (r, _) = self
                .app
                .hits
                .iter()
                .find(|(_, t)| want(t))
                .unwrap_or_else(|| panic!("no such target on {:?}", self.app.page))
                .clone();
            let k = self.app.k as i32;
            let p = Point::new((r.x + r.w / 2) * k, (r.y + r.h / 2) * k);
            self.run(Input::Move(p));
            self.run(Input::Press(p));
            self.run(Input::Release(p));
        }

        fn page(&mut self, page: Page) {
            self.click(|t| *t == Target::Page(page));
        }

        fn set(&mut self, key: &str, value: impl Into<toml::Value>) {
            let value = value.into();
            self.click(|t| matches!(t, Target::Set(k, v) if *k == key && *v == value));
        }

        fn type_text(&mut self, s: &str) {
            for c in s.chars() {
                self.run(Input::Text(c));
            }
        }

        fn effective(&self) -> config::Settings {
            Config::parse(self.text()).effective("owl", &Layer::default())
        }
    }

    #[test]
    fn a_click_writes_this_hosts_table_at_once() {
        let mut rig = Rig::new("# mine\nsize = 32\n");
        rig.type_text("otter");
        rig.set("mascot", "otter");
        assert_eq!(
            rig.text(),
            "# mine\nsize = 32\n\n[hosts.owl]\nmascot = \"otter\"\n"
        );
        assert_eq!(rig.written.len(), 1);
        assert_eq!(rig.effective().mascot.value, "otter");
        // The same choice again writes nothing.
        rig.set("mascot", "otter");
        assert_eq!(rig.written.len(), 1);
    }

    #[test]
    fn every_page_draws_and_every_key_is_on_one() {
        let mut rig = Rig::new("");
        let mut keys = std::collections::HashSet::new();
        for (page, _) in PAGES {
            rig.page(page);
            for (_, t) in &rig.app.hits {
                if let Target::Set(k, _) = t {
                    keys.insert(*k);
                }
                if let Target::Field(FieldId::OffsetX) = t {
                    keys.insert("offset");
                }
                if let Target::Field(FieldId::IntervalMin) = t {
                    keys.insert("interval");
                }
            }
        }
        for key in config::KEYS {
            assert!(keys.contains(key), "{key} has a control");
        }
    }

    #[test]
    fn scopes_choose_the_table_and_show_inheritance() {
        let mut rig = Rig::new("[hosts.\"o*\"]\nsize = 128\n");
        rig.page(Page::Look);
        let shown = rig.app.shown("size");
        assert_eq!(shown.value.as_integer(), Some(128));
        assert!(!shown.here);
        assert_eq!(shown.from, "from [hosts.o*]");

        rig.click(|t| *t == Target::Editing(Editing::AllHosts));
        rig.set("size", 32);
        assert!(rig.text().starts_with("size = 32\n"), "{}", rig.text());
        // Set at the top level, but the glob wins for this host.
        let shown = rig.app.shown("size");
        assert!(shown.here);
        assert_eq!(
            shown.shadow.as_deref(),
            Some("owl uses 128 from [hosts.o*]")
        );

        rig.click(|t| *t == Target::Editing(Editing::ThisHost));
        rig.set("size", 64);
        assert_eq!(rig.effective().size.value, 64);
        rig.click(|t| *t == Target::Clear("size"));
        assert_eq!(rig.effective().size.value, 128);
        assert!(
            !rig.text().contains("[hosts.owl]"),
            "tidied: {}",
            rig.text()
        );
    }

    #[test]
    fn number_fields_type_step_and_keep_intervals_valid() {
        let mut rig = Rig::new("");
        rig.page(Page::Motion);
        rig.click(|t| *t == Target::Field(FieldId::IntervalMin));
        for _ in 0..3 {
            rig.run(Input::Key(UiKey::Backspace));
        }
        rig.type_text("30");
        rig.run(Input::Key(UiKey::Enter));
        assert_eq!(rig.effective().interval.value, [30, 30], "max follows min");
        rig.click(|t| *t == Target::Step(FieldId::IntervalMax, 1));
        assert_eq!(rig.effective().interval.value, [30, 31]);

        rig.page(Page::Look);
        rig.click(|t| *t == Target::Field(FieldId::Size));
        rig.run(Input::Key(UiKey::Backspace));
        rig.run(Input::Key(UiKey::Backspace));
        rig.type_text("50");
        // Clicking elsewhere finishes the field too.
        rig.click(|t| *t == Target::Page(Page::Look));
        assert_eq!(rig.effective().size.value, 64, "50 is refused");
        assert!(
            rig.app
                .status
                .as_ref()
                .unwrap()
                .0
                .contains("multiple of 32")
        );
        rig.click(|t| *t == Target::Step(FieldId::Size, 32));
        assert_eq!(rig.effective().size.value, 96);

        rig.page(Page::Place);
        rig.click(|t| *t == Target::Step(FieldId::OffsetY, -1));
        assert_eq!(rig.effective().offset.value, [16, 15]);
    }

    #[test]
    fn labels_accents_and_displays_are_typed() {
        let mut rig = Rig::new("");
        rig.page(Page::Label);
        rig.set("label", "");
        rig.type_text("Build Box");
        rig.run(Input::Key(UiKey::Enter));
        assert_eq!(rig.effective().label.value, "Build Box");
        rig.set("label", "none");
        assert_eq!(rig.effective().label.value, "none");

        rig.page(Page::Look);
        rig.click(|t| *t == Target::Field(FieldId::Accent));
        rig.type_text("#12ab34");
        rig.run(Input::Key(UiKey::Enter));
        assert_eq!(rig.effective().accent.value, "#12ab34");
        rig.set("accent", "teal");
        assert_eq!(rig.effective().accent.value, "teal");

        rig.page(Page::Place);
        rig.set("display", "DP-1");
        assert_eq!(rig.effective().display.value, "DP-1");
        rig.set("position", "bottom-left");
        assert_eq!(rig.effective().position.value, Anchor::BottomLeft);
    }

    #[test]
    fn host_tables_are_managed_and_edited() {
        let mut rig = Rig::new("");
        rig.page(Page::Hosts);
        rig.click(|t| *t == Target::HostAdd);
        rig.type_text("web-*");
        rig.run(Input::Key(UiKey::Enter));
        assert_eq!(rig.app.editing, Editing::Table("web-*".into()));
        assert!(rig.text().contains("[hosts.\"web-*\"]"));
        rig.page(Page::System);
        rig.set("tray", false);
        let cfg = Config::parse(rig.text());
        assert!(!cfg.effective("web-01", &Layer::default()).tray.value);
        assert!(cfg.effective("owl", &Layer::default()).tray.value);

        rig.page(Page::Hosts);
        rig.click(|t| matches!(t, Target::HostRename(_)));
        for _ in 0..5 {
            rig.run(Input::Key(UiKey::Backspace));
        }
        rig.type_text("db-*");
        rig.run(Input::Key(UiKey::Enter));
        assert_eq!(rig.app.editing, Editing::Table("db-*".into()));
        assert_eq!(Config::parse(rig.text()).host_keys, ["db-*"]);

        rig.click(|t| matches!(t, Target::HostDelete(_)));
        assert!(rig.text().contains("db-*"), "asks first");
        rig.click(|t| matches!(t, Target::HostDelete(_)));
        assert!(
            Config::parse(rig.text()).host_keys.is_empty(),
            "{}",
            rig.text()
        );
        assert_eq!(rig.app.editing, Editing::ThisHost);
    }

    #[test]
    fn undo_walks_back_and_a_failed_write_reverts() {
        let mut rig = Rig::new("");
        rig.page(Page::Motion);
        rig.set("animation", "static");
        rig.page(Page::System);
        rig.set("fullscreen", "show");
        rig.run(Input::Shortcut('z'));
        assert_eq!(rig.effective().fullscreen.value, config::Fullscreen::Hide);
        rig.click(|t| *t == Target::Undo);
        assert_eq!(rig.text(), "");
        assert_eq!(rig.written.last().map(String::as_str), Some(""));

        rig.set("backend", "x11");
        rig.app.failed("disk full");
        assert_eq!(rig.text(), "");
        assert!(rig.app.status.as_ref().unwrap().0.contains("disk full"));
    }

    #[test]
    fn a_broken_file_is_never_written() {
        let mut rig = Rig::new("size = [");
        rig.page(Page::Motion);
        assert!(
            !rig.app
                .hits
                .iter()
                .any(|(_, t)| matches!(t, Target::Set(..)))
        );
        rig.run(Input::Shortcut('z'));
        assert!(rig.written.is_empty());
        // Fixed on disk: editable again.
        rig.app.reload("size = 32\n".into());
        rig.page(Page::Motion);
        rig.set("animation", "static");
        assert_eq!(rig.written.len(), 1);
    }

    #[test]
    fn typing_on_the_mascot_page_searches() {
        let mut rig = Rig::new("");
        rig.type_text("moos");
        let picks: Vec<String> = rig
            .app
            .hits
            .iter()
            .filter_map(|(_, t)| match t {
                Target::Set("mascot", v) => v.as_str().map(str::to_string),
                _ => None,
            })
            .filter(|m| m != "auto")
            .collect();
        assert_eq!(picks, ["moose"]);
        rig.run(Input::Key(UiKey::Enter));
        assert_eq!(
            rig.effective().mascot.value,
            "moose",
            "Enter picks the match"
        );
        rig.type_text("o");
        rig.run(Input::Key(UiKey::Escape));
        assert!(!rig.closed, "Esc clears the search first");
        rig.run(Input::Key(UiKey::Escape));
        assert!(rig.closed);
    }

    #[test]
    fn hovering_a_choice_previews_it_without_writing() {
        let mut rig = Rig::new("");
        rig.type_text("moose");
        let before = rig.app.presented.clone();
        let (r, _) = rig
            .app
            .hits
            .iter()
            .find(|(_, t)| matches!(t, Target::Set("mascot", v) if v.as_str() == Some("moose")))
            .unwrap()
            .clone();
        let k = rig.app.k as i32;
        rig.run(Input::Move(Point::new((r.x + 5) * k, (r.y + 5) * k)));
        // Twice: the first draw finds the target, the second previews it.
        rig.run(Input::Move(Point::new((r.x + 6) * k, (r.y + 6) * k)));
        assert_ne!(rig.app.presented, before);
        assert!(rig.written.is_empty());
    }

    #[test]
    fn the_window_fits_small_and_scaled_displays() {
        let mut app = App::new(
            Arc::new(Catalog::bundled().clone()),
            bundle(),
            Inputs {
                hostname: "owl".into(),
                text: String::new(),
                state: State::default(),
                path: String::new(),
            },
        );
        let retina = DisplayInfo {
            scale: 2.0,
            ..display()
        };
        assert_eq!(
            app.start(vec![retina])[0],
            Effect::Place(PhysicalRect::new(240, 16, 1440, 1008))
        );
        let small = DisplayInfo {
            work_area: PhysicalRect::new(0, 0, 800, 560),
            scale: 2.0,
            ..display()
        };
        let fx = app.displays_changed(vec![small]);
        assert_eq!(fx[0], Effect::Place(PhysicalRect::new(40, 28, 720, 504)));
    }
}
