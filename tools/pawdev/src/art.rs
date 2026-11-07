//! Bridges between the text format and pixel-art tools (SPEC §12.7): the
//! master palette as a GIMP/Aseprite `.gpl` file, PNG export of a mascot's
//! layers and frames, and PNG import back into a grid.

use std::path::Path;

use pawprint_core::catalog::bundle;
use pawprint_core::sprite::{Library, Origin, Rgb, RgbaImage, source_layers};

/// `assets/palette.gpl`, generated from `assets/palette.toml` in file order.
pub fn palette_gpl(palette_toml: &str) -> Result<String, String> {
    let table: toml::Table = toml::from_str(palette_toml).map_err(|e| e.to_string())?;
    let colors = table
        .get("colors")
        .and_then(|c| c.as_table())
        .ok_or("palette.toml has no [colors] table")?;
    let mut out = String::from(
        "GIMP Palette\nName: PAWPRINT master palette\nColumns: 8\n# Generated from assets/palette.toml by `make palette`. Do not edit.\n",
    );
    for (name, hex) in colors {
        let rgb = hex
            .as_str()
            .and_then(Rgb::parse_hex)
            .ok_or_else(|| format!("{name}: not #rrggbb"))?;
        out.push_str(&format!("{:>3} {:>3} {:>3}\t{name}\n", rgb.0, rgb.1, rgb.2));
    }
    Ok(out)
}

pub fn write_palette_gpl(root: &Path) -> Result<(), String> {
    let assets = root.join("assets");
    let src = std::fs::read_to_string(assets.join("palette.toml")).map_err(|e| e.to_string())?;
    let path = assets.join("palette.gpl");
    std::fs::write(&path, palette_gpl(&src)?).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn mascot_source(root: &Path, id: &str) -> Result<String, String> {
    let path = root.join("assets/mascots").join(id).join("mascot.toml");
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn library(root: &Path) -> Result<Library, String> {
    Library::load(&root.join("assets")).map_err(|r| {
        r.errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn report_text(r: &pawprint_core::sprite::Report) -> String {
    r.errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `out/art/<id>/`: one PNG per layer and variant at the layer's own size
/// (named `slot.png` or `slot.variant.png`), plus `frames.png`, a strip of
/// every distinct compiled 32×32 frame (rest first).
pub fn export(root: &Path, id: &str) -> Result<(), String> {
    let lib = library(root)?;
    let layers = source_layers(&mascot_source(root, id)?, &lib, Origin::Bundled)
        .map_err(|r| report_text(&r))?;
    let dir = root.join("out/art").join(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for l in &layers.layers {
        let mut img = RgbaImage::new(l.width as u32, l.height as u32);
        for (i, p) in l.pixels.iter().enumerate() {
            if let Some(c) = p {
                img.put(
                    (i % l.width) as i64,
                    (i / l.width) as i64,
                    [c.0, c.1, c.2, 255],
                );
            }
        }
        let path = dir.join(format!("{}.png", l.name));
        std::fs::write(&path, img.encode_png()).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}  (at [{}, {}])", path.display(), l.x, l.y);
    }
    let m = bundle()
        .mascot(id)
        .ok_or_else(|| format!("{id} is not in the bundle; run `make check-assets`"))?;
    let mut strip = RgbaImage::new(32 * m.frames.len() as u32, 32);
    for f in 0..m.frames.len() {
        let img = m.frame_rgba(f);
        for y in 0..32 {
            for x in 0..32 {
                strip.put(i64::from(f as u32 * 32 + x), i64::from(y), img.pixel(x, y));
            }
        }
    }
    let path = dir.join("frames.png");
    std::fs::write(&path, strip.encode_png()).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {} ({} frames)", path.display(), m.frames.len());
    Ok(())
}

/// The nearest master palette color by squared RGB distance.
fn nearest(lib: &Library, c: Rgb) -> (&str, Rgb) {
    let d = |p: &Rgb| {
        let f = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        f(p.0, c.0) + f(p.1, c.1) + f(p.2, c.2)
    };
    lib.palette
        .iter()
        .min_by_key(|(_, p)| d(p))
        .map(|(n, p)| (n.as_str(), *p))
        .expect("the master palette is not empty")
}

/// Convert a PNG back into a grid using the mascot's glyphs and print it as a
/// ready-to-paste `[layers."<name>"]` table. The layer name is the file name
/// without `.png`, as `art-export` wrote it; a known layer keeps its `at`.
pub fn import(root: &Path, id: &str, png: &Path) -> Result<(), String> {
    let lib = library(root)?;
    let layers = source_layers(&mascot_source(root, id)?, &lib, Origin::Bundled)
        .map_err(|r| report_text(&r))?;
    let bytes = std::fs::read(png).map_err(|e| format!("{}: {e}", png.display()))?;
    let img = RgbaImage::decode_png(&bytes)?;
    let (grid, notes) = to_grid(&img, &layers.palette, &lib)?;

    let name = png
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".png").to_string())
        .unwrap_or_default();
    let at = layers
        .layers
        .iter()
        .find(|l| l.name == name)
        .map(|l| (l.x, l.y));
    for n in &notes {
        eprintln!("note: {n}");
    }
    println!("[layers.\"{name}\"]");
    match at {
        Some((x, y)) => println!("at = [{x}, {y}]"),
        None if (img.width, img.height) == (32, 32) => {}
        None => println!("at = [0, 0]   # new layer: set its position"),
    }
    println!("grid = \"\"\"\n{grid}\"\"\"");
    Ok(())
}

/// Map pixels to glyphs. Transparent pixels take the glyph mapped to
/// `clear`; opaque ones snap to the nearest master palette color, then to the
/// mascot glyph for that color. Returns the grid and human notes about
/// inexact or unmapped colors.
fn to_grid(
    img: &RgbaImage,
    palette: &[(char, String)],
    lib: &Library,
) -> Result<(String, Vec<String>), String> {
    let glyph_for = |name: &str| palette.iter().find(|(_, v)| v == name).map(|(g, _)| *g);
    let clear = glyph_for("clear").unwrap_or('.');
    let mut notes = Vec::new();
    let mut grid = String::new();
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.pixel(x, y);
            let g = match p[3] {
                0 => clear,
                255 => {
                    let c = Rgb(p[0], p[1], p[2]);
                    let (name, snapped) = nearest(lib, c);
                    if snapped != c {
                        let n = format!("{} snapped to {name} ({})", c.to_hex(), snapped.to_hex());
                        if !notes.contains(&n) {
                            notes.push(n);
                        }
                    }
                    match glyph_for(name).or_else(|| glyph_for(&snapped.to_hex())) {
                        Some(g) => g,
                        None => {
                            let n = format!(
                                "{name} has no glyph in this mascot's [palette]; add one and replace '?'"
                            );
                            if !notes.contains(&n) {
                                notes.push(n);
                            }
                            '?'
                        }
                    }
                }
                a => {
                    return Err(format!(
                        "pixel ({x}, {y}) has alpha {a}; every pixel must be fully opaque or fully transparent"
                    ));
                }
            };
            grid.push(g);
        }
        grid.push('\n');
    }
    Ok((grid, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_gpl_is_up_to_date() {
        let root = crate::root();
        let src = std::fs::read_to_string(root.join("assets/palette.toml")).unwrap();
        let committed = std::fs::read_to_string(root.join("assets/palette.gpl"))
            .expect("assets/palette.gpl exists; run `make palette`");
        assert_eq!(
            committed.replace("\r\n", "\n"),
            palette_gpl(&src).unwrap(),
            "assets/palette.gpl is stale; run `make palette`"
        );
        assert_eq!(committed.lines().filter(|l| l.contains('\t')).count(), 32);
    }

    #[test]
    fn import_snaps_to_mascot_glyphs() {
        let lib = library(&crate::root()).unwrap();
        let ink = lib.palette["ink"];
        let mut img = RgbaImage::new(3, 1);
        img.put(0, 0, [ink.0, ink.1, ink.2 + 1, 255]);
        img.put(2, 0, [0xa8, 0x78, 0x4d, 255]); // brown-2, not in this palette
        let palette = vec![('.', "clear".to_string()), ('k', "ink".to_string())];
        let (grid, notes) = to_grid(&img, &palette, &lib).unwrap();
        assert_eq!(grid, "k.?\n");
        assert_eq!(notes.len(), 2, "{notes:?}");
        img.put(1, 0, [0, 0, 0, 128]);
        assert!(to_grid(&img, &palette, &lib).is_err());
    }
}
