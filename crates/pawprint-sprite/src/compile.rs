//! Mascot compiler: source → indexed frames plus actions (SPEC §12.3–12.7,
//! validation per §12.11).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::grid::parse_grid;
use crate::library::single_char;
use crate::source::{ActionSrc, FrameSrc, LayerSrc, MascotFile, StepSrc, When};
use crate::{
    CATEGORIES, FRAME_SIZE, Library, Origin, Report, Rgb, RgbaImage, is_kebab, is_valid_id, squash,
};

const N: usize = FRAME_SIZE as usize;
/// Default frame duration: 5 FPS (SPEC §7.1).
pub const DEFAULT_MS: u32 = 200;
pub const MIN_MS: u32 = 80;
pub const MAX_MS: u32 = 2000;
pub const MAX_ACTION_MS: u32 = 4000;
/// Weight of an action that doesn't set one.
pub const DEFAULT_WEIGHT: u32 = 5;
/// Rows that must show something at rest, so the mascot peeks out of the
/// crate (SPEC §4.2).
const VISIBLE_ROWS: usize = 20;
const MAX_COLORS_WARN: usize = 16;

/// A compiled mascot. Frames are 32 × 32 palette indices: 0 is transparent,
/// `i` is `colors[i - 1]`. `frames[0]` is the rest frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mascot {
    pub id: String,
    pub name: String,
    pub category: String,
    pub aliases: Vec<String>,
    pub accent: String,
    pub fallback: bool,
    pub nocturnal: bool,
    /// The base it extends, for grouping in the contact sheet.
    pub base: Option<String>,
    pub colors: Vec<Rgb>,
    pub frames: Vec<Vec<u8>>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub weight: u32,
    pub when: When,
    /// The motion template it came from; `None` for a signature action.
    pub template: Option<String>,
    pub frames: Vec<FrameRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRef {
    pub frame: u16,
    pub ms: u16,
}

impl Mascot {
    /// Expand one frame to RGBA.
    pub fn frame_rgba(&self, index: usize) -> RgbaImage {
        let mut img = RgbaImage::new(FRAME_SIZE, FRAME_SIZE);
        for (i, &p) in self.frames[index].iter().enumerate() {
            if p != 0 {
                let c = self.colors[usize::from(p) - 1];
                img.put((i % N) as i64, (i / N) as i64, [c.0, c.1, c.2, 255]);
            }
        }
        img
    }
}

/// The result of compiling one mascot. `mascot` is `Some` only when there
/// are no errors.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub mascot: Option<Mascot>,
    pub report: Report,
}

/// Compile `mascot.toml` in `dir`. The directory name must equal the id.
pub fn compile_dir(dir: &Path, lib: &Library, origin: Origin) -> Compiled {
    let dir_name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
    let src = match std::fs::read_to_string(dir.join("mascot.toml")) {
        Ok(s) => s,
        Err(e) => {
            let mut report = Report::default();
            report.error("mascot.toml", format!("cannot read: {e}"));
            return Compiled {
                mascot: None,
                report,
            };
        }
    };
    let read =
        |name: &str| std::fs::read(dir.join(name)).map_err(|e| format!("cannot read {name}: {e}"));
    compile(&src, dir_name.as_deref(), lib, origin, &read)
}

/// Compile mascot source text. `read_file` loads files the source refers to
/// (the PNG strip of a PNG-mode mascot).
pub fn compile(
    src: &str,
    dir_name: Option<&str>,
    lib: &Library,
    origin: Origin,
    read_file: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Compiled {
    let mut report = Report::default();
    let mascot = match toml::from_str::<MascotFile>(src) {
        Ok(file) => compile_file(file, dir_name, lib, origin, read_file, &mut report),
        Err(e) => {
            report.error("", e.to_string());
            None
        }
    };
    let mascot = if report.errors.is_empty() {
        mascot
    } else {
        None
    };
    Compiled { mascot, report }
}

fn compile_file(
    file: MascotFile,
    dir_name: Option<&str>,
    lib: &Library,
    origin: Origin,
    read_file: &dyn Fn(&str) -> Result<Vec<u8>, String>,
    report: &mut Report,
) -> Option<Mascot> {
    let identity = Identity::check(&file, dir_name, lib, report);
    let base = file.extends.clone();
    let mut out = Interner::default();
    let actions = if let Some(sprite) = file.sprite.clone() {
        compile_png(file, sprite, read_file, &mut out, report)?
    } else {
        let file = match &file.extends {
            Some(name) => match lib.bases.get(name) {
                Some(b) => merge_base(b.clone(), file),
                None => {
                    report.error("extends", format!("no base named {name:?}"));
                    return None;
                }
            },
            None => file,
        };
        compile_text(file, lib, origin, &mut out, report)?
    };

    if actions.is_empty() {
        report.error("actions", "a mascot needs at least one action");
    }
    if origin == Origin::Bundled && !actions.iter().any(|a| a.template.is_none()) {
        report.error(
            "actions",
            "bundled mascots need a signature (non-template) action",
        );
    }
    let rest = &out.frames[0];
    if !rest[..VISIBLE_ROWS * N].iter().any(|&p| p != 0) {
        report.error(
            "rest",
            format!(
                "the rest frame shows nothing in rows 0-{}",
                VISIBLE_ROWS - 1
            ),
        );
    }
    if out.colors.len() > MAX_COLORS_WARN {
        report.warn(
            "",
            format!(
                "uses {} colors; the limit is {MAX_COLORS_WARN}",
                out.colors.len()
            ),
        );
    }

    let identity = identity?;
    Some(Mascot {
        id: identity.id,
        name: identity.name,
        category: identity.category,
        aliases: identity.aliases,
        accent: identity.accent,
        fallback: identity.fallback,
        nocturnal: identity.nocturnal,
        base,
        colors: out.colors,
        frames: out.frames,
        actions,
    })
}

struct Identity {
    id: String,
    name: String,
    category: String,
    aliases: Vec<String>,
    accent: String,
    fallback: bool,
    nocturnal: bool,
}

impl Identity {
    fn check(
        file: &MascotFile,
        dir_name: Option<&str>,
        lib: &Library,
        report: &mut Report,
    ) -> Option<Identity> {
        let start = report.errors.len();
        let id = file.id.clone().unwrap_or_default();
        if !is_valid_id(&id) {
            report.error(
                "id",
                format!("{id:?} is not a valid id (lowercase kebab-case, at most 32 characters)"),
            );
        }
        if let Some(dir) = dir_name
            && dir != id
        {
            report.error(
                "id",
                format!("{id:?} must match its directory name {dir:?}"),
            );
        }
        let name = file.name.clone().unwrap_or_default();
        if name.trim().is_empty() {
            report.error("name", "a mascot needs a name");
        }
        let category = file.category.clone().unwrap_or_default();
        if !CATEGORIES.contains(&category.as_str()) {
            report.error(
                "category",
                format!("{category:?} is not one of {}", CATEGORIES.join(", ")),
            );
        }
        for alias in &file.aliases {
            if squash(alias).is_empty() {
                report.error(
                    "aliases",
                    format!("alias {alias:?} has no letters or digits"),
                );
            }
        }
        let accent = file.accent.clone().unwrap_or_default();
        if lib.accent(&accent).is_none() && Rgb::parse_hex(&accent).is_none() {
            report.error(
                "accent",
                format!("{accent:?} is not an accent name or #rrggbb"),
            );
        }
        (report.errors.len() == start).then(|| Identity {
            id,
            name,
            category,
            aliases: file.aliases.clone(),
            accent,
            fallback: file.fallback.unwrap_or(true),
            nocturnal: file.nocturnal.unwrap_or(false),
        })
    }
}

/// Merge a mascot over its base: palette entries override, layers with the
/// same name replace in place and new layers append, actions and frames add.
fn merge_base(mut base: MascotFile, m: MascotFile) -> MascotFile {
    base.palette.extend(m.palette);
    for (name, layer) in m.layers {
        base.layers.insert(name, layer);
    }
    base.actions.extend(m.actions);
    base.frames.extend(m.frames);
    MascotFile {
        id: m.id,
        name: m.name,
        category: m.category,
        aliases: m.aliases,
        accent: m.accent,
        fallback: m.fallback,
        nocturnal: m.nocturnal,
        extends: m.extends,
        sprite: None,
        palette: base.palette,
        layers: base.layers,
        rest: m.rest.or(base.rest),
        actions: base.actions,
        frames: base.frames,
    }
}

type Canvas = Vec<Option<Rgb>>;

/// Collects distinct colors and frames.
#[derive(Default)]
struct Interner {
    colors: Vec<Rgb>,
    frames: Vec<Vec<u8>>,
    seen: HashMap<Vec<u8>, u16>,
}

impl Interner {
    fn frame(&mut self, canvas: &Canvas, report: &mut Report) -> u16 {
        let mut px = Vec::with_capacity(N * N);
        for c in canvas {
            px.push(match c {
                None => 0,
                Some(rgb) => match self.colors.iter().position(|x| x == rgb) {
                    Some(i) => (i + 1) as u8,
                    None if self.colors.len() < 255 => {
                        self.colors.push(*rgb);
                        self.colors.len() as u8
                    }
                    None => {
                        report.error("", "more than 255 colors");
                        0
                    }
                },
            });
        }
        if let Some(&i) = self.seen.get(&px) {
            return i;
        }
        let i = self.frames.len() as u16;
        self.seen.insert(px.clone(), i);
        self.frames.push(px);
        i
    }
}

// ---------------------------------------------------------------- text mode

#[derive(Debug)]
struct LayerDef {
    slot: String,
    variant: Option<String>,
    x: i32,
    y: i32,
    width: usize,
    pixels: Canvas,
}

/// Everything a pose is rendered from.
struct Model<'a> {
    slots: Vec<String>,
    layers: Vec<LayerDef>,
    grids: BTreeMap<String, Canvas>,
    lib: &'a Library,
}

#[derive(Debug, Clone, Default)]
struct SlotPose {
    visible: bool,
    variant: Option<String>,
    dx: i32,
    dy: i32,
}

#[derive(Debug, Clone)]
struct Pose {
    slots: Vec<SlotPose>,
    all: (i32, i32),
    flipx: bool,
    props: Vec<(String, i32, i32)>,
    grid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Variant {
        slot: String,
        variant: String,
    },
    Offset {
        slot: String,
        dx: i32,
        dy: i32,
    },
    All {
        dx: i32,
        dy: i32,
    },
    Hide(String),
    Show {
        slot: String,
        variant: Option<String>,
    },
    Prop {
        name: String,
        x: i32,
        y: i32,
    },
    FlipX,
}

fn parse_xy(s: &str) -> Option<(i32, i32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn split_layer_name(name: &str) -> (String, Option<String>) {
    match name.split_once('.') {
        Some((slot, variant)) => (slot.to_string(), Some(variant.to_string())),
        None => (name.to_string(), None),
    }
}

/// Parse one layer op (SPEC §12.3). Offsets are signed: `head+-1,0`.
fn parse_op(s: &str) -> Result<Op, String> {
    let bad = || format!("cannot parse op {s:?}");
    if s == "flipx" {
        return Ok(Op::FlipX);
    }
    if let Some(rest) = s.strip_prefix("prop:") {
        let (name, xy) = rest.split_once('@').ok_or_else(bad)?;
        let (x, y) = parse_xy(xy).ok_or_else(bad)?;
        return Ok(Op::Prop {
            name: name.to_string(),
            x,
            y,
        });
    }
    if let Some(slot) = s.strip_prefix('-') {
        return Ok(Op::Hide(slot.to_string()));
    }
    if let Some(name) = s.strip_prefix('+') {
        let (slot, variant) = split_layer_name(name);
        return Ok(Op::Show { slot, variant });
    }
    if let Some((slot, variant)) = s.split_once('=') {
        return Ok(Op::Variant {
            slot: slot.to_string(),
            variant: variant.to_string(),
        });
    }
    if let Some((slot, xy)) = s.split_once('+') {
        let (dx, dy) = parse_xy(xy).ok_or_else(bad)?;
        return Ok(if slot == "all" {
            Op::All { dx, dy }
        } else {
            Op::Offset {
                slot: slot.to_string(),
                dx,
                dy,
            }
        });
    }
    Err(bad())
}

impl Model<'_> {
    fn slot(&self, name: &str) -> Result<usize, String> {
        self.slots
            .iter()
            .position(|s| s == name)
            .ok_or_else(|| format!("no layer slot {name:?}"))
    }

    fn layer(&self, slot: &str, variant: Option<&str>) -> Option<&LayerDef> {
        self.layers
            .iter()
            .find(|l| l.slot == slot && l.variant.as_deref() == variant)
    }

    fn require_layer(&self, slot: &str, variant: Option<&str>) -> Result<(), String> {
        self.slot(slot)?;
        match self.layer(slot, variant) {
            Some(_) => Ok(()),
            None => match variant {
                Some(v) => Err(format!("no layer variant {slot}.{v}")),
                None => Err(format!(
                    "slot {slot:?} has no default layer; show a variant like +{slot}.<variant>"
                )),
            },
        }
    }

    fn apply(&self, pose: &mut Pose, op: &Op) -> Result<(), String> {
        let slot_op = matches!(
            op,
            Op::Variant { .. } | Op::Offset { .. } | Op::Hide(_) | Op::Show { .. }
        );
        if slot_op && pose.grid.is_some() {
            return Err("grid frames take only all+, flipx, and prop: ops".into());
        }
        match op {
            Op::Variant { slot, variant } => {
                self.require_layer(slot, Some(variant))?;
                let s = &mut pose.slots[self.slot(slot)?];
                s.visible = true;
                s.variant = Some(variant.clone());
            }
            Op::Show { slot, variant } => {
                self.require_layer(slot, variant.as_deref())?;
                let s = &mut pose.slots[self.slot(slot)?];
                s.visible = true;
                s.variant = variant.clone();
            }
            Op::Hide(slot) => pose.slots[self.slot(slot)?].visible = false,
            Op::Offset { slot, dx, dy } => {
                let s = &mut pose.slots[self.slot(slot)?];
                s.dx += dx;
                s.dy += dy;
            }
            Op::All { dx, dy } => {
                pose.all.0 += dx;
                pose.all.1 += dy;
            }
            Op::Prop { name, x, y } => {
                if !self.lib.props.contains_key(name) {
                    return Err(format!("no prop named {name:?}"));
                }
                pose.props.push((name.clone(), *x, *y));
            }
            Op::FlipX => pose.flipx = !pose.flipx,
        }
        Ok(())
    }

    fn render(&self, pose: &Pose) -> Canvas {
        let mut canvas: Canvas = vec![None; N * N];
        let mut blit = |pixels: &Canvas, width: usize, x0: i32, y0: i32| {
            for (i, p) in pixels.iter().enumerate() {
                let (x, y) = (x0 + (i % width) as i32, y0 + (i / width) as i32);
                if let Some(c) = p
                    && (0..N as i32).contains(&x)
                    && (0..N as i32).contains(&y)
                {
                    canvas[y as usize * N + x as usize] = Some(*c);
                }
            }
        };
        if let Some(grid) = &pose.grid {
            blit(&self.grids[grid], N, pose.all.0, pose.all.1);
        } else {
            for (i, slot) in self.slots.iter().enumerate() {
                let s = &pose.slots[i];
                if let Some(layer) = self.layer(slot, s.variant.as_deref()).filter(|_| s.visible) {
                    blit(
                        &layer.pixels,
                        layer.width,
                        layer.x + s.dx + pose.all.0,
                        layer.y + s.dy + pose.all.1,
                    );
                }
            }
        }
        if pose.flipx {
            for row in canvas.chunks_mut(N) {
                row.reverse();
            }
        }
        for (name, x, y) in &pose.props {
            let prop = &self.lib.props[name];
            for (i, p) in prop.pixels.iter().enumerate() {
                let (px, py) = (x + (i % prop.width) as i32, y + (i / prop.width) as i32);
                if let Some(c) = p
                    && (0..N as i32).contains(&px)
                    && (0..N as i32).contains(&py)
                {
                    canvas[py as usize * N + px as usize] = Some(*c);
                }
            }
        }
        canvas
    }
}

fn resolve_color(value: &str, lib: &Library, origin: Origin) -> Result<Option<Rgb>, String> {
    if value == "clear" {
        return Ok(None);
    }
    if let Some(c) = lib.palette.get(value) {
        return Ok(Some(*c));
    }
    match (origin, Rgb::parse_hex(value)) {
        (Origin::User, Some(c)) => Ok(Some(c)),
        (Origin::User, None) => Err(format!("{value:?} is not a master palette name or #rrggbb")),
        (Origin::Bundled, _) => Err(format!("{value:?} is not a master palette name")),
    }
}

fn grid_pixels(
    text: &str,
    glyphs: &HashMap<char, Option<Rgb>>,
) -> Result<(usize, usize, Canvas), String> {
    let grid = parse_grid(text)?;
    let mut pixels = Vec::with_capacity(grid.glyphs.len());
    for g in &grid.glyphs {
        pixels.push(
            *glyphs
                .get(g)
                .ok_or_else(|| format!("glyph {g:?} is not in the palette"))?,
        );
    }
    Ok((grid.width, grid.height, pixels))
}

fn compile_text(
    file: MascotFile,
    lib: &Library,
    origin: Origin,
    out: &mut Interner,
    report: &mut Report,
) -> Option<Vec<Action>> {
    let start = report.errors.len();

    let mut glyphs = HashMap::new();
    for (key, value) in &file.palette {
        match (single_char(key), resolve_color(value, lib, origin)) {
            (Ok(c), Ok(rgb)) => {
                glyphs.insert(c, rgb);
            }
            (Err(e), _) | (_, Err(e)) => report.error("palette", e),
        }
    }

    let mut model = Model {
        slots: Vec::new(),
        layers: Vec::new(),
        grids: BTreeMap::new(),
        lib,
    };
    for (name, value) in file.layers {
        let ctx = format!("layer {name}");
        let (slot, variant) = split_layer_name(&name);
        if !is_kebab(&slot) || slot == "all" || variant.as_deref().is_some_and(|v| !is_kebab(v)) {
            report.error(
                &ctx,
                "layer names are kebab-case `slot` or `slot.variant`, and `all` is reserved",
            );
            continue;
        }
        let src: LayerSrc = match value.try_into() {
            Ok(s) => s,
            Err(e) => {
                report.error(&ctx, e.to_string());
                continue;
            }
        };
        let (width, height, pixels) = match grid_pixels(&src.grid, &glyphs) {
            Ok(g) => g,
            Err(e) => {
                report.error(&ctx, e);
                continue;
            }
        };
        let [x, y] = src.at.unwrap_or([0, 0]);
        if src.at.is_none() && (width, height) != (N, N) {
            report.error(
                &ctx,
                format!("without `at`, a layer is a full {N}x{N} grid (got {width}x{height})"),
            );
        } else if x < 0 || y < 0 || x as usize + width > N || y as usize + height > N {
            report.error(
                &ctx,
                format!("a {width}x{height} grid at [{x}, {y}] does not fit in {N}x{N}"),
            );
        }
        if !model.slots.contains(&slot) {
            model.slots.push(slot.clone());
        }
        model.layers.push(LayerDef {
            slot,
            variant,
            x,
            y,
            width,
            pixels,
        });
    }

    for (name, frame) in &file.frames {
        match grid_pixels(&frame.grid, &glyphs) {
            Ok((w, h, px)) if (w, h) == (N, N) => {
                model.grids.insert(name.clone(), px);
            }
            Ok((w, h, _)) => report.error(
                format!("frame {name}"),
                format!("custom frames are {N}x{N} (got {w}x{h})"),
            ),
            Err(e) => report.error(format!("frame {name}"), e),
        }
    }

    let rest = rest_pose(&model, file.rest.as_ref(), report);
    if report.errors.len() != start {
        return None;
    }
    let rest_frame = model.render(&rest);
    out.frame(&rest_frame, report);

    let mut actions = Vec::new();
    for (i, a) in file.actions.iter().enumerate() {
        if let Some(action) = text_action(i, a, &model, &rest, out, report) {
            actions.push(action);
        }
    }
    (report.errors.len() == start).then_some(actions)
}

fn rest_pose(model: &Model<'_>, rest: Option<&toml::Value>, report: &mut Report) -> Pose {
    let mut pose = Pose {
        slots: model
            .slots
            .iter()
            .map(|s| SlotPose {
                visible: model.layer(s, None).is_some(),
                ..SlotPose::default()
            })
            .collect(),
        all: (0, 0),
        flipx: false,
        props: Vec::new(),
        grid: None,
    };
    let Some(rest) = rest else { return pose };
    let names: Option<Vec<&str>> = rest
        .as_array()
        .and_then(|a| a.iter().map(toml::Value::as_str).collect());
    let Some(names) = names else {
        report.error("rest", "in text mascots `rest` is a list of layer names");
        return pose;
    };
    for s in &mut pose.slots {
        s.visible = false;
    }
    for name in names {
        let (slot, variant) = split_layer_name(name);
        if let Err(e) = model.apply(&mut pose, &Op::Show { slot, variant }) {
            report.error("rest", e);
        }
    }
    pose
}

fn text_action(
    i: usize,
    a: &ActionSrc,
    model: &Model<'_>,
    rest: &Pose,
    out: &mut Interner,
    report: &mut Report,
) -> Option<Action> {
    let label = a
        .name
        .clone()
        .or(a.template.clone())
        .unwrap_or_else(|| format!("#{}", i + 1));
    let ctx = format!("action {label}");
    if a.duration_ms.is_some() {
        report.error(
            &ctx,
            "`duration_ms` is for PNG mascots; use `ms` on each frame",
        );
    }
    let (name, weight, when, steps) = match &a.template {
        Some(t) => {
            let Some(m) = model.lib.motions.get(t) else {
                report.error(&ctx, format!("no motion template named {t:?}"));
                return None;
            };
            if a.frames.is_some() {
                report.error(
                    &ctx,
                    "an action that uses a template cannot also list frames",
                );
            }
            for need in &m.needs {
                let (slot, variant) = split_layer_name(need);
                if model.layer(&slot, variant.as_deref()).is_none() {
                    report.error(&ctx, format!("template {t:?} needs layer {need:?}"));
                }
            }
            let steps = m.frames.iter().cloned().map(FrameSrc::Step).collect();
            (
                a.name.clone().unwrap_or(m.name.clone()),
                a.weight.or(m.weight),
                a.when.or(m.when),
                steps,
            )
        }
        None => {
            let Some(name) = a.name.clone() else {
                report.error(&ctx, "an action needs `name` (or `use` for a template)");
                return None;
            };
            let Some(frames) = a.frames.clone() else {
                report.error(&ctx, "an action needs `frames`");
                return None;
            };
            (name, a.weight, a.when, frames)
        }
    };

    let mut frames = Vec::new();
    for (j, step) in steps.iter().enumerate() {
        let step = match step {
            FrameSrc::Step(s) => s,
            FrameSrc::Index(_) => {
                report.error(
                    &ctx,
                    "frame indices are for PNG mascots; text frames are { ops = [...], ms = ... }",
                );
                return None;
            }
        };
        match render_step(step, model, rest) {
            Ok(canvas) => frames.push((out.frame(&canvas, report), step.ms.unwrap_or(DEFAULT_MS))),
            Err(e) => report.error(format!("{ctx}, frame {}", j + 1), e),
        }
    }
    finish_action(&ctx, name, weight, when, a.template.clone(), frames, report)
}

fn render_step(step: &StepSrc, model: &Model<'_>, rest: &Pose) -> Result<Canvas, String> {
    let mut pose = rest.clone();
    if let Some(grid) = &step.grid {
        if !model.grids.contains_key(grid) {
            return Err(format!("no custom frame {grid:?}"));
        }
        pose.grid = Some(grid.clone());
    }
    for op in &step.ops {
        model.apply(&mut pose, &parse_op(op)?)?;
    }
    Ok(model.render(&pose))
}

/// Shared timing and weight checks for both modes (SPEC §12.11 rule 5).
fn finish_action(
    ctx: &str,
    name: String,
    weight: Option<u32>,
    when: Option<When>,
    template: Option<String>,
    frames: Vec<(u16, u32)>,
    report: &mut Report,
) -> Option<Action> {
    let start = report.errors.len();
    let weight = weight.unwrap_or(DEFAULT_WEIGHT);
    if weight == 0 {
        report.error(ctx, "weight must be at least 1");
    }
    if frames.is_empty() {
        report.error(ctx, "an action needs at least one frame");
    }
    for &(_, ms) in &frames {
        if !(MIN_MS..=MAX_MS).contains(&ms) {
            report.error(
                ctx,
                format!("frame duration {ms} ms is outside {MIN_MS}-{MAX_MS} ms"),
            );
        }
    }
    let total: u32 = frames.iter().map(|&(_, ms)| ms).sum();
    if total > MAX_ACTION_MS {
        report.error(
            ctx,
            format!("lasts {total} ms; an action is at most {MAX_ACTION_MS} ms"),
        );
    }
    (report.errors.len() == start).then(|| Action {
        name,
        weight,
        when: when.unwrap_or_default(),
        template,
        frames: frames
            .into_iter()
            .map(|(frame, ms)| FrameRef {
                frame,
                ms: ms as u16,
            })
            .collect(),
    })
}

// ----------------------------------------------------------------- PNG mode

fn compile_png(
    file: MascotFile,
    sprite: String,
    read_file: &dyn Fn(&str) -> Result<Vec<u8>, String>,
    out: &mut Interner,
    report: &mut Report,
) -> Option<Vec<Action>> {
    let start = report.errors.len();
    if file.extends.is_some()
        || !file.palette.is_empty()
        || !file.layers.is_empty()
        || !file.frames.is_empty()
    {
        report.error(
            "sprite",
            "PNG mascots take no extends, palette, layers, or frames",
        );
    }
    let img = match read_file(&sprite).and_then(|b| RgbaImage::decode_png(&b)) {
        Ok(img) => img,
        Err(e) => {
            report.error("sprite", e);
            return None;
        }
    };
    if img.height != FRAME_SIZE || img.width == 0 || img.width % FRAME_SIZE != 0 {
        report.error(
            "sprite",
            format!(
                "must be a strip of {N}x{N} frames (got {}x{})",
                img.width, img.height
            ),
        );
        return None;
    }
    if img.data.chunks(4).any(|p| p[3] != 0 && p[3] != 255) {
        report.error(
            "sprite",
            "every pixel must be fully opaque or fully transparent",
        );
        return None;
    }
    let count = (img.width / FRAME_SIZE) as usize;
    let strip: Vec<Canvas> = (0..count)
        .map(|f| {
            (0..N * N)
                .map(|i| {
                    let p = img.pixel((f * N + i % N) as u32, (i / N) as u32);
                    (p[3] == 255).then_some(Rgb(p[0], p[1], p[2]))
                })
                .collect()
        })
        .collect();

    let rest = match &file.rest {
        None => 0,
        Some(v) => match v.as_integer().and_then(|i| usize::try_from(i).ok()) {
            Some(i) if i < count => i,
            _ => {
                report.error(
                    "rest",
                    format!("in PNG mascots `rest` is a frame index below {count}"),
                );
                return None;
            }
        },
    };
    out.frame(&strip[rest], report);

    let mut actions = Vec::new();
    for (i, a) in file.actions.iter().enumerate() {
        let ctx = format!(
            "action {}",
            a.name.clone().unwrap_or_else(|| format!("#{}", i + 1))
        );
        if a.template.is_some() {
            report.error(&ctx, "PNG mascots cannot use motion templates");
            continue;
        }
        let Some(name) = a.name.clone() else {
            report.error(&ctx, "an action needs `name`");
            continue;
        };
        let ms = a.duration_ms.unwrap_or(DEFAULT_MS);
        let mut frames = Vec::new();
        for f in a.frames.iter().flatten() {
            match f {
                FrameSrc::Index(n) if (*n as usize) < count => {
                    frames.push((out.frame(&strip[*n as usize], report), ms))
                }
                FrameSrc::Index(n) => report.error(
                    &ctx,
                    format!("frame {n} is past the end of the {count}-frame strip"),
                ),
                FrameSrc::Step(_) => {
                    report.error(&ctx, "PNG mascot frames are indices into the sprite strip")
                }
            }
        }
        if let Some(action) = finish_action(&ctx, name, a.weight, a.when, None, frames, report) {
            actions.push(action);
        }
    }
    (report.errors.len() == start).then_some(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::lib_with;

    fn no_files(name: &str) -> Result<Vec<u8>, String> {
        Err(format!("no file {name}"))
    }

    /// 32 rows of 32 glyphs: `fill` on rows `rows`, `.` elsewhere.
    fn full(fill: char, rows: std::ops::Range<usize>) -> String {
        (0..N).map(|y| if rows.contains(&y) { fill.to_string().repeat(N) } else { ".".repeat(N) } + "\n").collect()
    }

    const BLINK: (&str, &str) = (
        "blink",
        "name = \"blink\"\nneeds = [\"eyes.closed\"]\nframes = [{ ops = [\"eyes=closed\"], ms = 140 }]",
    );
    const ZZZ: (&str, &str) = (
        "zzz",
        "name = \"zzz\"\npalette = { z = \"white\" }\ngrid = \"zz\"",
    );

    fn owl_src(extra: &str) -> String {
        format!(
            r#"
id = "owl"
name = "Owl"
category = "birds"
aliases = ["hoot"]
accent = "violet"

[palette]
"." = "clear"
"k" = "ink"
"e" = "amber"
"f" = "brown-2"

[layers.body]
grid = """
{body}"""

[layers.eyes]
at = [2, 2]
grid = """
ee.ee
"""

[layers."eyes.closed"]
at = [2, 2]
grid = """
kk.kk
"""

[[actions]]
use = "blink"
weight = 6

[[actions]]
name = "nudge"
frames = [{{ ops = ["eyes+1,0"], ms = 200 }}, {{ ops = ["all+0,1", "flipx"] }}]
{extra}
"#,
            body = full('f', 0..10)
        )
    }

    fn compile_owl(extra: &str) -> Compiled {
        let lib = lib_with(&[], &[BLINK], &[ZZZ]);
        compile(
            &owl_src(extra),
            Some("owl"),
            &lib,
            Origin::Bundled,
            &no_files,
        )
    }

    fn errors(c: &Compiled) -> String {
        c.report
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn px(m: &Mascot, frame: usize, x: usize, y: usize) -> Option<Rgb> {
        let p = m.frames[frame][y * N + x];
        (p != 0).then(|| m.colors[usize::from(p) - 1])
    }

    const AMBER: Rgb = Rgb(0xf0, 0xa8, 0x30);
    const INK: Rgb = Rgb(0x1b, 0x1b, 0x1f);
    const BROWN: Rgb = Rgb(0xa8, 0x78, 0x4d);

    #[test]
    fn compiles_layers_variants_and_templates() {
        let c = compile_owl("");
        assert!(c.report.is_clean(), "{}", errors(&c));
        let m = c.mascot.unwrap();
        assert_eq!(px(&m, 0, 2, 2), Some(AMBER));
        assert_eq!(px(&m, 0, 4, 2), Some(BROWN)); // transparent eye gap shows the body
        assert_eq!(px(&m, 0, 0, 12), None);
        let blink = &m.actions[0];
        assert_eq!(
            (blink.name.as_str(), blink.weight, blink.template.as_deref()),
            ("blink", 6, Some("blink"))
        );
        assert_eq!(blink.frames, vec![FrameRef { frame: 1, ms: 140 }]);
        assert_eq!(px(&m, 1, 2, 2), Some(INK));
    }

    #[test]
    fn applies_offsets_all_and_flipx() {
        let m = compile_owl("").mascot.unwrap();
        let nudge = &m.actions[1];
        assert_eq!(nudge.template, None);
        assert_eq!(nudge.weight, DEFAULT_WEIGHT);
        let [f1, f2] = [
            nudge.frames[0].frame as usize,
            nudge.frames[1].frame as usize,
        ];
        // eyes+1,0: eye pixel moves from x=2 to x=3.
        assert_eq!(px(&m, f1, 2, 2), Some(BROWN));
        assert_eq!(px(&m, f1, 3, 2), Some(AMBER));
        // all+0,1 then flipx: row 0 empties, eye at (31-2, 3).
        assert_eq!(px(&m, f2, 5, 0), None);
        assert_eq!(px(&m, f2, 29, 3), Some(AMBER));
        assert_eq!(nudge.frames[1].ms, DEFAULT_MS as u16);
    }

    #[test]
    fn identical_frames_are_shared() {
        let m = compile_owl(
            "[[actions]]\nname = \"still\"\nframes = [{ ops = [] }, { ops = [\"eyes=closed\"] }]",
        )
        .mascot
        .unwrap();
        let still = &m.actions[2];
        assert_eq!(
            still.frames[0].frame, 0,
            "an empty-op frame is the rest frame"
        );
        assert_eq!(still.frames[1].frame, m.actions[0].frames[0].frame);
    }

    #[test]
    fn props_hide_show_and_custom_grids() {
        let extra = format!(
            "[[actions]]\nname = \"sleep\"\nwhen = \"night\"\nframes = [{{ ops = [\"-eyes\", \"prop:zzz@0,20\"] }}, {{ grid = \"curl\" }}]\n\n[frames.curl]\ngrid = \"\"\"\n{}\"\"\"",
            full('k', 5..6)
        );
        let c = compile_owl(&extra);
        assert!(c.report.errors.is_empty(), "{}", errors(&c));
        let m = c.mascot.unwrap();
        let sleep = &m.actions[2];
        assert_eq!(sleep.when, When::Night);
        let (f1, f2) = (
            sleep.frames[0].frame as usize,
            sleep.frames[1].frame as usize,
        );
        assert_eq!(px(&m, f1, 2, 2), Some(BROWN), "eyes hidden");
        assert_eq!(
            px(&m, f1, 1, 20),
            Some(Rgb(0xf7, 0xf5, 0xef)),
            "zzz prop drawn"
        );
        assert_eq!(px(&m, f2, 0, 5), Some(INK));
        assert_eq!(px(&m, f2, 0, 0), None);
    }

    #[test]
    fn optional_slots_and_explicit_rest() {
        let extra = "[[actions]]\nname = \"yap\"\nframes = [{ ops = [\"+mouth.open\"] }, { ops = [\"+eyes\"] }]\n[layers.\"mouth.open\"]\nat = [0, 5]\ngrid = \"k\"";
        let src = owl_src(extra).replacen(
            "accent = \"violet\"",
            "accent = \"violet\"\nrest = [\"body\", \"eyes.closed\"]",
            1,
        );
        let lib = lib_with(&[], &[BLINK], &[ZZZ]);
        let c = compile(&src, Some("owl"), &lib, Origin::Bundled, &no_files);
        assert!(c.report.errors.is_empty(), "{}", errors(&c));
        let m = c.mascot.unwrap();
        assert_eq!(px(&m, 0, 2, 2), Some(INK), "rest uses closed eyes");
        assert_eq!(px(&m, 0, 0, 5), Some(BROWN), "mouth hidden at rest");
        let yap = &m.actions[2];
        assert_eq!(px(&m, yap.frames[0].frame as usize, 0, 5), Some(INK));
        assert_eq!(px(&m, yap.frames[1].frame as usize, 2, 2), Some(AMBER));
    }

    #[test]
    fn extends_a_base_with_recolor() {
        let base = format!(
            "[palette]\n\".\" = \"clear\"\n\"f\" = \"brown-2\"\n\"e\" = \"amber\"\n[layers.body]\ngrid = \"\"\"\n{}\"\"\"\n[layers.eyes]\nat = [1, 1]\ngrid = \"e\"\n[layers.\"eyes.closed\"]\nat = [1, 1]\ngrid = \"f\"\n",
            full('f', 0..8)
        );
        let lib = lib_with(&[("perch", &base)], &[BLINK], &[]);
        let src = "id = \"crow\"\nname = \"Crow\"\ncategory = \"birds\"\naccent = \"#112233\"\nextends = \"perch\"\n[palette]\n\"f\" = \"fur-black\"\n[layers.beak]\nat = [3, 3]\ngrid = \"e\"\n[[actions]]\nuse = \"blink\"\n[[actions]]\nname = \"caw\"\nframes = [{ ops = [\"beak+1,0\"] }]";
        let c = compile(src, Some("crow"), &lib, Origin::Bundled, &no_files);
        assert!(c.report.is_clean(), "{}", errors(&c));
        let m = c.mascot.unwrap();
        assert_eq!(m.base.as_deref(), Some("perch"));
        assert_eq!(
            px(&m, 0, 0, 0),
            Some(Rgb(0x2b, 0x28, 0x33)),
            "base body recolored"
        );
        assert_eq!(px(&m, 0, 1, 1), Some(AMBER));
        assert_eq!(
            px(&m, 0, 3, 3),
            Some(AMBER),
            "added layer drawn after base layers"
        );
    }

    #[test]
    fn validation_errors() {
        let cases: &[(&str, &str, &str)] = &[
            ("bad id", "id = \"owl\"", "id = \"Owl\""),
            ("unknown glyph", "ee.ee", "ee?ee"),
            ("ragged", "ee.ee\n", "ee.ee\nee\n"),
            ("missing slot", "\"eyes+1,0\"", "\"wings+1,0\""),
            (
                "missing variant",
                "{ ops = [\"all+0,1\"",
                "{ ops = [\"eyes=wide\"",
            ),
            ("bad template", "use = \"blink\"", "use = \"wink\""),
            ("too short", "ms = 200 }", "ms = 20 }"),
            (
                "unknown accent",
                "accent = \"violet\"",
                "accent = \"mauve\"",
            ),
            (
                "bad category",
                "category = \"birds\"",
                "category = \"fowl\"",
            ),
            ("hex in bundled", "\"e\" = \"amber\"", "\"e\" = \"#f0a830\""),
            ("bad op", "\"eyes+1,0\"", "\"eyes*2\""),
            (
                "unknown field",
                "name = \"Owl\"",
                "name = \"Owl\"\ncolour = 1",
            ),
            (
                "off canvas",
                "at = [2, 2]\ngrid = \"\"\"\nee.ee",
                "at = [30, 2]\ngrid = \"\"\"\nee.ee",
            ),
        ];
        for (what, from, to) in cases {
            let src = owl_src("").replacen(from, to, 1);
            assert_ne!(src, owl_src(""), "{what}: replacement did not apply");
            let lib = lib_with(&[], &[BLINK], &[ZZZ]);
            let c = compile(&src, Some("owl"), &lib, Origin::Bundled, &no_files);
            assert!(
                c.mascot.is_none() && !c.report.errors.is_empty(),
                "{what}: expected an error"
            );
        }
    }

    #[test]
    fn dir_name_must_match_id() {
        let lib = lib_with(&[], &[BLINK], &[]);
        let c = compile(&owl_src(""), Some("hoot"), &lib, Origin::Bundled, &no_files);
        assert!(errors(&c).contains("directory name"));
    }

    #[test]
    fn template_needs_are_checked() {
        let src = owl_src("").replace("[layers.\"eyes.closed\"]", "[layers.\"eyes.shut\"]");
        let lib = lib_with(&[], &[BLINK], &[]);
        let c = compile(&src, Some("owl"), &lib, Origin::Bundled, &no_files);
        assert!(
            errors(&c).contains("needs layer \"eyes.closed\""),
            "{}",
            errors(&c)
        );
    }

    #[test]
    fn bundled_needs_signature_and_action_limits() {
        let lib = lib_with(&[], &[BLINK], &[]);
        let only_template = owl_src("")
            .replace("name = \"nudge\"", "use = \"blink\"\nname = \"nudge2\"")
            .replace(
                "frames = [{ ops = [\"eyes+1,0\"], ms = 200 }, { ops = [\"all+0,1\", \"flipx\"] }]",
                "",
            );
        let c = compile(
            &only_template,
            Some("owl"),
            &lib,
            Origin::Bundled,
            &no_files,
        );
        assert!(errors(&c).contains("signature"), "{}", errors(&c));
        let user = compile(&only_template, Some("owl"), &lib, Origin::User, &no_files);
        assert!(
            user.mascot.is_some(),
            "user mascots need no signature: {}",
            errors(&user)
        );

        let long = owl_src(
            "[[actions]]\nname = \"long\"\nframes = [{ ms = 2000 }, { ms = 2000 }, { ms = 200 }]",
        );
        assert!(
            errors(&compile(
                &long,
                Some("owl"),
                &lib,
                Origin::Bundled,
                &no_files
            ))
            .contains("at most 4000")
        );
    }

    #[test]
    fn rest_must_peek_out_of_the_crate() {
        let src = owl_src("")
            .replace(&full('f', 0..10), &full('f', 25..30))
            .replace("at = [2, 2]", "at = [2, 22]");
        let lib = lib_with(&[], &[BLINK], &[]);
        let c = compile(&src, Some("owl"), &lib, Origin::Bundled, &no_files);
        assert!(errors(&c).contains("rows 0-19"), "{}", errors(&c));
    }

    #[test]
    fn too_many_colors_warns() {
        let src = "id = \"x\"\nname = \"X\"\ncategory = \"oddballs\"\naccent = \"#000000\"\nsprite = \"s.png\"\n[[actions]]\nname = \"a\"\nframes = [0]\n";
        let mut img = RgbaImage::new(32, 32);
        for i in 0..20u8 {
            img.put(i64::from(i), 0, [i * 10, 0, 0, 255]);
        }
        let png = img.encode_png();
        let lib = lib_with(&[], &[], &[]);
        let c = compile(src, Some("x"), &lib, Origin::User, &|_| Ok(png.clone()));
        assert!(c.mascot.is_some(), "{}", errors(&c));
        assert!(
            c.report
                .warnings
                .iter()
                .any(|w| w.message.contains("20 colors")),
            "{:?}",
            c.report.warnings
        );
    }

    fn png_strip(frames: u32, alpha: u8) -> Vec<u8> {
        let mut img = RgbaImage::new(32 * frames, 32);
        for f in 0..frames {
            img.put(i64::from(f * 32 + f), 0, [255, 0, 0, alpha]);
        }
        img.encode_png()
    }

    const PNG_SRC: &str = "id = \"my-owl\"\nname = \"My Owl\"\ncategory = \"birds\"\naliases = [\"hooter\"]\naccent = \"violet\"\nsprite = \"sprite.png\"\nrest = 0\n\n[[actions]]\nname = \"blink\"\nframes = [1, 0]\nduration_ms = 200\n";

    #[test]
    fn png_mode_mascot() {
        let lib = lib_with(&[], &[], &[]);
        let strip = png_strip(2, 255);
        let c = compile(PNG_SRC, Some("my-owl"), &lib, Origin::User, &|name| {
            assert_eq!(name, "sprite.png");
            Ok(strip.clone())
        });
        assert!(c.report.is_clean(), "{}", errors(&c));
        let m = c.mascot.unwrap();
        assert_eq!(m.frames.len(), 2);
        assert_eq!(
            m.actions[0].frames,
            vec![
                FrameRef { frame: 1, ms: 200 },
                FrameRef { frame: 0, ms: 200 }
            ]
        );
        assert_eq!(px(&m, 0, 0, 0), Some(Rgb(255, 0, 0)));
        assert_eq!(px(&m, 1, 1, 0), Some(Rgb(255, 0, 0)));
        assert_eq!(m.frame_rgba(1).pixel(1, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn png_mode_errors() {
        let lib = lib_with(&[], &[], &[]);
        let run = |src: &str, png: Vec<u8>| {
            errors(&compile(src, Some("my-owl"), &lib, Origin::User, &|_| {
                Ok(png.clone())
            }))
        };
        assert!(run(PNG_SRC, png_strip(2, 128)).contains("fully opaque"));
        assert!(run(PNG_SRC, png_strip(1, 255)).contains("past the end"));
        assert!(run(PNG_SRC, RgbaImage::new(33, 32).encode_png()).contains("strip of 32x32"));
        assert!(run(&PNG_SRC.replace("rest = 0", "rest = 5"), png_strip(2, 255)).contains("rest"));
        let missing = compile(PNG_SRC, Some("my-owl"), &lib, Origin::User, &no_files);
        assert!(errors(&missing).contains("no file sprite.png"));
    }

    #[test]
    fn op_parsing() {
        let cases = [
            (
                "eyes=closed",
                Op::Variant {
                    slot: "eyes".into(),
                    variant: "closed".into(),
                },
            ),
            (
                "head+1,0",
                Op::Offset {
                    slot: "head".into(),
                    dx: 1,
                    dy: 0,
                },
            ),
            (
                "ear-left+-1,2",
                Op::Offset {
                    slot: "ear-left".into(),
                    dx: -1,
                    dy: 2,
                },
            ),
            ("all+0,-2", Op::All { dx: 0, dy: -2 }),
            ("-tail", Op::Hide("tail".into())),
            (
                "+mouth.open",
                Op::Show {
                    slot: "mouth".into(),
                    variant: Some("open".into()),
                },
            ),
            (
                "+tail",
                Op::Show {
                    slot: "tail".into(),
                    variant: None,
                },
            ),
            (
                "prop:zzz@20,1",
                Op::Prop {
                    name: "zzz".into(),
                    x: 20,
                    y: 1,
                },
            ),
            ("flipx", Op::FlipX),
        ];
        for (s, want) in cases {
            assert_eq!(parse_op(s), Ok(want), "{s}");
        }
        for bad in ["head", "head+1", "prop:zzz", "head+a,b"] {
            assert!(parse_op(bad).is_err(), "{bad}");
        }
    }
}
