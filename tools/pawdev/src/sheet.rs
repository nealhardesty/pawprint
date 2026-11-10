//! Review sheets (SPEC §12.6): the contact sheet of every mascot
//! (`docs/gallery.png`, optionally `docs/gallery.gif`) and the building-blocks
//! sheet of every base, template, prop, and accent (`docs/building-blocks.png`).
//! Captions use PAWPRINT's own 5×7 font.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use pawprint_core::catalog::bundle;
use pawprint_core::layout::LabelSize;
use pawprint_core::render::{BadgeSpec, render_badge, scale};
use pawprint_core::sprite::{Mascot, Metrics, Rgb, RgbaImage, compile_base};

const BG: Rgb = Rgb(0xe6, 0xe2, 0xd8);
const INK: Rgb = Rgb(0x1b, 0x1b, 0x1f);
const DIM: Rgb = Rgb(0x8a, 0x86, 0x7e);
const PAD: u32 = 16;

// ------------------------------------------------------------------ drawing

fn fill(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: Rgb) {
    for yy in y..y + h {
        for xx in x..x + w {
            img.put(i64::from(xx), i64::from(yy), [c.0, c.1, c.2, 255]);
        }
    }
}

fn blit(img: &mut RgbaImage, src: &RgbaImage, x: u32, y: u32) {
    for sy in 0..src.height {
        for sx in 0..src.width {
            let p = src.pixel(sx, sy);
            if p[3] != 0 {
                img.put(i64::from(x + sx), i64::from(y + sy), p);
            }
        }
    }
}

/// Text in the 5×7 font at an integer scale. Returns the width drawn.
fn text(img: &mut RgbaImage, s: &str, x: u32, y: u32, k: u32, c: Rgb) -> u32 {
    let font = &bundle().font;
    for (i, ch) in s.chars().enumerate() {
        let gx = x + i as u32 * font.metrics.advance * k;
        for fy in 0..font.metrics.height {
            for fx in 0..font.metrics.width {
                if font.ink(ch, fx, fy) {
                    fill(img, gx + fx * k, y + fy * k, k, k, c);
                }
            }
        }
    }
    text_width(s, k)
}

fn text_width(s: &str, k: u32) -> u32 {
    Metrics::REGULAR.text_width(s.chars().count()) * k
}

fn accent_of(m: &Mascot) -> Rgb {
    let b = bundle();
    b.accents
        .iter()
        .find(|(n, _)| *n == m.accent)
        .map(|(_, c)| *c)
        .or_else(|| Rgb::parse_hex(&m.accent))
        .unwrap_or(b.accents[0].1)
}

/// The badge without a label (labels are hidden on review sheets).
fn badge(m: &Mascot, frame: usize, size: u32, accent: Rgb) -> RgbaImage {
    let b = bundle();
    let spec = BadgeSpec {
        mascot: m,
        frame,
        label: None,
        accent,
        size,
        label_size: LabelSize::Auto,
        display_scale: 1.0,
    };
    render_badge(&spec, b)
}

fn write_png(path: &Path, img: &RgbaImage) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, img.encode_png()).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {} ({}x{})", path.display(), img.width, img.height);
    Ok(())
}

// ------------------------------------------------------------ contact sheet

const COLS: u32 = 6;
const CELL_W: u32 = 32 + 6 + 64;
const CELL_H: u32 = 64 + 5 + 7;
const GAP_X: u32 = 22;
const GAP_Y: u32 = 14;
const HEAD_H: u32 = 7 + 4 + 1 + 8;

/// Mascots grouped by base: bases alphabetically, bespoke mascots last.
fn groups() -> Vec<(String, Vec<&'static Mascot>)> {
    let mut by_base: BTreeMap<(bool, String), Vec<&Mascot>> = BTreeMap::new();
    for m in &bundle().mascots {
        let key = match &m.base {
            Some(b) => (false, b.clone()),
            None => (true, "bespoke".to_string()),
        };
        by_base.entry(key).or_default().push(m);
    }
    by_base
        .into_iter()
        .map(|((_, name), ms)| (name, ms))
        .collect()
}

struct Layout {
    width: u32,
    height: u32,
    /// Top-left of each mascot's cell, in catalog order of `groups()`.
    cells: Vec<(&'static Mascot, u32, u32)>,
    headings: Vec<(String, u32)>,
}

fn layout() -> Layout {
    let width = PAD * 2 + COLS * CELL_W + (COLS - 1) * GAP_X;
    let mut y = PAD + 14 + 6 + 7 + 14;
    let mut cells = Vec::new();
    let mut headings = Vec::new();
    for (name, ms) in groups() {
        headings.push((format!("{} ({})", name.to_uppercase(), ms.len()), y));
        y += HEAD_H;
        for (i, m) in ms.iter().enumerate() {
            let (col, row) = (i as u32 % COLS, i as u32 / COLS);
            cells.push((*m, PAD + col * (CELL_W + GAP_X), y + row * (CELL_H + GAP_Y)));
        }
        let rows = (ms.len() as u32).div_ceil(COLS);
        y += rows * (CELL_H + GAP_Y) + 6;
    }
    Layout {
        width,
        height: y + PAD - 6,
        cells,
        headings,
    }
}

/// Draw one cell: the badge at 1× and 2×, bottoms aligned, and the id.
fn draw_cell(img: &mut RgbaImage, m: &Mascot, frame: usize, x: u32, y: u32) {
    fill(img, x, y, CELL_W, 64, BG);
    let accent = accent_of(m);
    blit(img, &badge(m, frame, 32, accent), x, y + 32);
    blit(img, &badge(m, frame, 64, accent), x + 32 + 6, y);
}

fn sheet_base(l: &Layout) -> RgbaImage {
    let mut img = RgbaImage::new(l.width, l.height);
    fill(&mut img, 0, 0, l.width, l.height, BG);
    text(&mut img, "PAWPRINT GALLERY", PAD, PAD, 2, INK);
    let n = bundle().mascots.len();
    let sub = format!(
        "{n} MASCOT{} - REST FRAMES AT 1X AND 2X - LABELS HIDDEN",
        if n == 1 { "" } else { "S" }
    );
    text(&mut img, &sub, PAD, PAD + 14 + 6, 1, DIM);
    for (h, y) in &l.headings {
        text(&mut img, h, PAD, *y, 1, INK);
        fill(&mut img, PAD, y + 7 + 4, l.width - 2 * PAD, 1, DIM);
    }
    for (m, x, y) in &l.cells {
        draw_cell(&mut img, m, 0, *x, *y);
        let w = text_width(&m.id, 1);
        let cx = x + CELL_W.saturating_sub(w) / 2;
        text(&mut img, &m.id, cx, y + 64 + 5, 1, INK);
    }
    img
}

pub fn contact_sheet(root: &Path, animated: bool) -> Result<(), String> {
    let l = layout();
    let img = sheet_base(&l);
    write_png(&root.join("docs/gallery.png"), &img)?;
    if animated {
        gallery_gif(root, &l, img)?;
    }
    Ok(())
}

/// Each mascot's loop: rest, then every action in turn with rest between.
fn timeline(m: &Mascot) -> Vec<(usize, u32)> {
    const REST_MS: u32 = 700;
    let mut t = vec![(0, REST_MS)];
    for a in &m.actions {
        t.extend(
            a.frames
                .iter()
                .map(|f| (usize::from(f.frame), u32::from(f.ms))),
        );
        t.push((0, REST_MS));
    }
    t
}

/// Frame shown at time `ms` (after the loop ends, rest).
fn frame_at(t: &[(usize, u32)], ms: u32) -> usize {
    let mut acc = 0;
    for &(f, d) in t {
        acc += d;
        if ms < acc {
            return f;
        }
    }
    0
}

fn gallery_gif(root: &Path, l: &Layout, first: RgbaImage) -> Result<(), String> {
    // The sheet is sampled on a 100 ms clock. A finer one (GIF allows 10 ms)
    // multiplies the frame count, because 100 mascots change at unrelated
    // times, and the shortest frames in the catalog are 80 ms anyway.
    const TICK_MS: u32 = 100;
    let lines: Vec<Vec<(usize, u32)>> = l.cells.iter().map(|(m, _, _)| timeline(m)).collect();
    let mut times: Vec<u32> = vec![0];
    for t in &lines {
        let mut acc = 0;
        for &(_, d) in t {
            acc += d;
            times.push(acc / TICK_MS * TICK_MS);
        }
    }
    times.sort_unstable();
    times.dedup();
    let end = *times.last().expect("has 0");

    let mut img = first;
    let mut shown: Vec<usize> = vec![0; l.cells.len()];
    // GIF frame k shows the sheet at times[k] until times[k + 1]. The first
    // is the whole sheet; later ones cover only the cells that changed, with
    // unchanged pixels transparent so they compress to almost nothing. The
    // state at `end` is all-rest again, which frame 0 already shows.
    let mut frames: Vec<((u32, u32, u32, u32), RgbaImage)> =
        vec![((0, 0, img.width, img.height), img.clone())];
    let delays: Vec<u16> = times
        .windows(2)
        .map(|w| ((w[1] - w[0]) / 10).max(1) as u16)
        .collect();
    for &now in &times[1..times.len() - 1] {
        let mut bbox: Option<(u32, u32, u32, u32)> = None;
        let before = img.clone();
        for (i, (m, x, y)) in l.cells.iter().enumerate() {
            let f = frame_at(&lines[i], now);
            if f != shown[i] {
                shown[i] = f;
                draw_cell(&mut img, m, f, *x, *y);
                let r = (*x, *y, x + CELL_W, y + 64);
                bbox = Some(match bbox {
                    None => r,
                    Some(b) => (b.0.min(r.0), b.1.min(r.1), b.2.max(r.2), b.3.max(r.3)),
                });
            }
        }
        let (x0, y0, x1, y1) = bbox.unwrap_or((0, 0, 1, 1));
        let mut sub = RgbaImage::new(x1 - x0, y1 - y0);
        for yy in 0..sub.height {
            for xx in 0..sub.width {
                let p = img.pixel(x0 + xx, y0 + yy);
                if p != before.pixel(x0 + xx, y0 + yy) {
                    sub.put(i64::from(xx), i64::from(yy), p);
                }
            }
        }
        frames.push(((x0, y0, x1 - x0, y1 - y0), sub));
    }

    // One global palette: index 0 is transparent, then every color used.
    let mut index: HashMap<[u8; 3], u8> = HashMap::new();
    let mut palette: Vec<u8> = vec![0, 0, 0];
    let mut indexed = Vec::new();
    for (_, sub) in &frames {
        let mut buf = Vec::with_capacity((sub.width * sub.height) as usize);
        for p in sub.data.chunks(4) {
            let key = [p[0], p[1], p[2]];
            let i = match index.get(&key) {
                _ if p[3] == 0 => 0,
                Some(&i) => i,
                None => {
                    if index.len() == 255 {
                        return Err("gallery uses more than 255 colors".into());
                    }
                    let i = index.len() as u8 + 1;
                    index.insert(key, i);
                    palette.extend_from_slice(&key);
                    i
                }
            };
            buf.push(i);
        }
        indexed.push(buf);
    }
    while palette.len() < 3 * 2 || !(palette.len() / 3).is_power_of_two() {
        palette.extend_from_slice(&[0, 0, 0]);
    }

    let path = root.join("docs/gallery.gif");
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let gif_err = |e: gif::EncodingError| format!("{}: {e}", path.display());
    let mut enc = gif::Encoder::new(
        std::io::BufWriter::new(file),
        l.width as u16,
        l.height as u16,
        &palette,
    )
    .map_err(gif_err)?;
    enc.set_repeat(gif::Repeat::Infinite).map_err(gif_err)?;
    for (((rect, _), buf), delay) in frames.iter().zip(&indexed).zip(&delays) {
        let frame = gif::Frame {
            delay: *delay,
            left: rect.0 as u16,
            top: rect.1 as u16,
            width: rect.2 as u16,
            height: rect.3 as u16,
            buffer: std::borrow::Cow::Borrowed(buf),
            transparent: Some(0),
            dispose: gif::DisposalMethod::Keep,
            ..gif::Frame::default()
        };
        enc.write_frame(&frame).map_err(gif_err)?;
    }
    drop(enc);
    println!(
        "wrote {} ({}x{}, {} frames, {:.1} s loop)",
        path.display(),
        l.width,
        l.height,
        frames.len(),
        f64::from(end) / 1000.0
    );
    Ok(())
}

// ---------------------------------------------------------- building blocks

/// The frame of an action that differs most from rest: what a still image
/// of the template should show.
fn key_frame(m: &Mascot, action: usize) -> usize {
    let rest = &m.frames[0];
    m.actions[action]
        .frames
        .iter()
        .map(|f| usize::from(f.frame))
        .max_by_key(|&f| m.frames[f].iter().zip(rest).filter(|(a, b)| a != b).count())
        .unwrap_or(0)
}

pub fn building_blocks(root: &Path) -> Result<(), String> {
    let b = bundle();
    let lib = b.library().map_err(|r| format!("{:?}", r.errors))?;
    let motions: Vec<&str> = lib.motion_names().collect();
    let mut bases = Vec::new();
    for (name, _) in &b.library.bases {
        let c = compile_base(&lib, name);
        let m = c
            .mascot
            .ok_or_else(|| format!("base {name}: {:?}", c.report.errors))?;
        bases.push((name.clone(), m));
    }

    const COL_W: u32 = 76;
    const LEFT: u32 = 104;
    const ROW_H: u32 = 64 + 10;
    let cols = 1 + motions.len() as u32;
    const PROP_K: u32 = 4;
    const PROP_H: u32 = 12 * PROP_K;
    let width = (PAD * 2 + LEFT + cols * COL_W).max(PAD * 2 + 16 * 72);
    let violet = b.accents[0].1;

    // Section heights.
    let title_h = 14 + 6 + 7 + 16;
    let accents_h = HEAD_H + 64 + 7 + 6 + 16;
    let props_h = HEAD_H + PROP_H + 7 + 6 + 16;
    let grid_h = HEAD_H + 7 + 6 + bases.len() as u32 * ROW_H;
    let height = PAD * 2 + title_h + accents_h + props_h + grid_h;
    let mut img = RgbaImage::new(width, height);
    fill(&mut img, 0, 0, width, height, BG);

    let heading = |img: &mut RgbaImage, s: &str, y: u32| {
        text(img, s, PAD, y, 1, INK);
        fill(img, PAD, y + 7 + 4, width - 2 * PAD, 1, DIM);
    };

    let mut y = PAD;
    text(&mut img, "PAWPRINT BUILDING BLOCKS", PAD, y, 2, INK);
    text(
        &mut img,
        "CRATE ACCENTS, PROPS, AND EVERY BASE WITH EVERY MOTION TEMPLATE (MOST-CHANGED FRAME, 2X)",
        PAD,
        y + 20,
        1,
        DIM,
    );
    y += title_h;

    // Accents: the crate in every accent, with the label strip in use.
    heading(&mut img, "CRATE ACCENTS (LABEL_SIZE SMALL AT SIZE 64)", y);
    y += HEAD_H;
    let owl = b.mascot("owl").or(b.mascots.first());
    let step = (width - 2 * PAD) / 16;
    for (i, (name, rgb)) in b.accents.iter().take(16).enumerate() {
        let x = PAD + i as u32 * step;
        if let Some(m) = owl {
            let spec = BadgeSpec {
                mascot: m,
                frame: 0,
                label: Some(&name.to_uppercase()),
                accent: *rgb,
                size: 64,
                label_size: LabelSize::Small,
                display_scale: 1.0,
            };
            blit(&mut img, &render_badge(&spec, b), x, y);
        }
    }
    y += 64 + 7 + 6 + 16;

    // Props at 4×, bottom-aligned.
    heading(&mut img, "PROPS (4X)", y);
    y += HEAD_H;
    let mut x = PAD;
    for (name, _) in &b.library.props {
        let p = &lib_prop(&lib, name);
        let big = scale(p, PROP_K);
        let w = big.width.max(text_width(name, 1));
        blit(
            &mut img,
            &big,
            x + (w - big.width) / 2,
            y + PROP_H - big.height.min(PROP_H),
        );
        text(
            &mut img,
            name,
            x + (w - text_width(name, 1)) / 2,
            y + PROP_H + 6,
            1,
            INK,
        );
        x += w + 18;
    }
    y += PROP_H + 7 + 6 + 16;

    // Bases × templates.
    heading(
        &mut img,
        "BASES X MOTION TEMPLATES ('-' = THE BASE LACKS A NEEDED LAYER)",
        y,
    );
    y += HEAD_H;
    let col_x = |c: u32| PAD + LEFT + c * COL_W;
    let names = std::iter::once("rest").chain(motions.iter().copied());
    for (c, name) in names.enumerate() {
        let w = text_width(name, 1);
        text(
            &mut img,
            name,
            col_x(c as u32) + (64u32.saturating_sub(w)) / 2,
            y,
            1,
            DIM,
        );
    }
    y += 7 + 6;
    for (name, m) in &bases {
        text(&mut img, name, PAD, y + 28, 1, INK);
        blit(&mut img, &badge(m, 0, 64, violet), col_x(0), y);
        for (c, motion) in motions.iter().enumerate() {
            let x = col_x(c as u32 + 1);
            match m
                .actions
                .iter()
                .position(|a| a.template.as_deref() == Some(motion))
            {
                Some(a) => blit(&mut img, &badge(m, key_frame(m, a), 64, violet), x, y),
                None => {
                    text(&mut img, "-", x + 29, y + 28, 1, DIM);
                }
            }
        }
        y += ROW_H;
    }
    write_png(&root.join("docs/building-blocks.png"), &img)
}

fn lib_prop(lib: &pawprint_core::sprite::Library, name: &str) -> RgbaImage {
    let p = lib.prop(name).expect("listed props exist");
    let mut img = RgbaImage::new(p.width as u32, p.height as u32);
    for (i, px) in p.pixels.iter().enumerate() {
        if let Some(c) = px {
            img.put(
                (i % p.width) as i64,
                (i / p.width) as i64,
                [c.0, c.1, c.2, 255],
            );
        }
    }
    img
}

// ------------------------------------------------------------ mascot sheet

const DARK: Rgb = Rgb(0x23, 0x26, 0x2b);

/// `make mascot-sheet DIR=<folder>`: every mascot folder in `dir`, compiled
/// with the user-mascot rules (SPEC §12.11) against the embedded library,
/// so work in progress never touches `assets/` or the build. Writes one
/// PNG per mascot into `out` (its rest frame at 1× and 2× on a light and a
/// dark desktop, then every action frame by frame at 2× with its time) and
/// `index.png` with every rest frame at 1× and 2×. Fails on any compile
/// error or warning, as the bundled build would.
pub fn mascot_sheet(dir: &Path, out: &Path) -> Result<(), String> {
    let lib = pawprint_core::catalog::library();
    let (mascots, warnings) = pawprint_core::catalog::load_user_mascots(dir, lib);
    for w in &warnings {
        println!("warning: {w}");
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    for m in &mascots {
        write_png(&out.join(format!("{}.png", m.id)), &mascot_strip(m))?;
    }
    if !mascots.is_empty() {
        write_png(&out.join("index.png"), &index_sheet(&mascots))?;
    }
    if warnings.is_empty() {
        println!("{} mascot(s), no warnings", mascots.len());
        Ok(())
    } else {
        Err(format!(
            "{} warning(s) or error(s); a bundled mascot must have none",
            warnings.len()
        ))
    }
}

fn mascot_strip(m: &Mascot) -> RgbaImage {
    const F: u32 = 64;
    const STEP: u32 = F + 6;
    let accent = accent_of(m);
    let longest = m.actions.iter().map(|a| a.frames.len()).max().unwrap_or(0) as u32;
    let width = (PAD * 2 + longest * STEP).max(PAD * 2 + 2 * (32 + 4 + F + 18) + 128);
    let row_h = F + 7 + 4 + 7 + 10;
    let height = PAD * 2 + 14 + 6 + (128 + 10) + m.actions.len() as u32 * row_h;
    let mut img = RgbaImage::new(width, height);
    fill(&mut img, 0, 0, width, height, BG);
    let title = format!("{} - {} - {}", m.id, m.name, m.category);
    text(&mut img, &title.to_uppercase(), PAD, PAD, 2, INK);
    // Rest: 1x and 2x on light, then on dark.
    let mut x = PAD;
    let y = PAD + 14 + 6;
    for bg in [BG, DARK] {
        fill(&mut img, x, y + 64, 32 + 4 + F + 8, F + 4, bg);
        blit(&mut img, &badge(m, 0, 32, accent), x + 2, y + 64 + 2 + 32);
        blit(
            &mut img,
            &badge(m, 0, F, accent),
            x + 2 + 32 + 4,
            y + 64 + 2,
        );
        x += 32 + 4 + F + 8 + 10;
    }
    // And at 4x, to read single pixels.
    blit(&mut img, &badge(m, 0, 128, accent), x, y);
    let mut y = y + 128 + 10;
    for a in &m.actions {
        let kind = match &a.template {
            Some(t) => format!("template {t}"),
            None => "own".to_string(),
        };
        let caption = format!("{}  ({kind}, weight {}, {:?})", a.name, a.weight, a.when);
        text(&mut img, &caption.to_uppercase(), PAD, y, 1, INK);
        for (i, f) in a.frames.iter().enumerate() {
            let fx = PAD + i as u32 * STEP;
            blit(
                &mut img,
                &badge(m, usize::from(f.frame), F, accent),
                fx,
                y + 11,
            );
            text(&mut img, &format!("{}MS", f.ms), fx, y + 11 + F + 2, 1, DIM);
        }
        y += row_h;
    }
    img
}

fn index_sheet(mascots: &[Mascot]) -> RgbaImage {
    const COLS: u32 = 8;
    const W: u32 = 32 + 4 + 64;
    const H: u32 = 64 + 4 + 7;
    let rows = (mascots.len() as u32).div_ceil(COLS);
    let width = PAD * 2 + COLS * (W + 14);
    let height = PAD * 2 + rows * (H + 10);
    let mut img = RgbaImage::new(width, height);
    fill(&mut img, 0, 0, width, height, BG);
    for (i, m) in mascots.iter().enumerate() {
        let (col, row) = (i as u32 % COLS, i as u32 / COLS);
        let (x, y) = (PAD + col * (W + 14), PAD + row * (H + 10));
        let accent = accent_of(m);
        blit(&mut img, &badge(m, 0, 32, accent), x, y + 32);
        blit(&mut img, &badge(m, 0, 64, accent), x + 36, y);
        text(&mut img, &m.id, x, y + 64 + 4, 1, INK);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_cycles_actions_and_ends_at_rest() {
        let m = &bundle().mascots[0];
        let t = timeline(m);
        assert_eq!(t.first().map(|f| f.0), Some(0));
        assert_eq!(t.last().map(|f| f.0), Some(0));
        let total: u32 = t.iter().map(|f| f.1).sum();
        assert_eq!(frame_at(&t, total + 5), 0);
    }

    #[test]
    fn layout_places_every_mascot_once() {
        let l = layout();
        assert_eq!(l.cells.len(), bundle().mascots.len());
        for (_, x, y) in &l.cells {
            assert!(x + CELL_W <= l.width && y + CELL_H <= l.height);
        }
    }
}
