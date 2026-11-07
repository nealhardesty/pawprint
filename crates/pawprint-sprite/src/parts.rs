//! The two non-mascot art sources: the bitmap font and the crate.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::grid::parse_grid;
use crate::library::single_char;
use crate::source::{CrateFile, FontFile};
use crate::{Library, Report};

/// PAWPRINT's 5 × 7 bitmap font for printable ASCII (SPEC §4.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Font {
    /// One entry per character 0x20..=0x7E. Each row's bit 4 is the leftmost
    /// column.
    pub glyphs: Vec<[u8; 7]>,
}

impl Font {
    pub const WIDTH: u32 = 5;
    pub const HEIGHT: u32 = 7;
    pub const ADVANCE: u32 = 6;
    const FIRST: u32 = 0x20;
    const LAST: u32 = 0x7e;

    /// The glyph for `c`. Characters outside printable ASCII render as `?`.
    pub fn glyph(&self, c: char) -> &[u8; 7] {
        let code = if (Self::FIRST..=Self::LAST).contains(&(c as u32)) {
            c as u32
        } else {
            '?' as u32
        };
        &self.glyphs[(code - Self::FIRST) as usize]
    }

    /// Is the pixel at (x, y) of glyph `c` inked?
    pub fn ink(&self, c: char, x: u32, y: u32) -> bool {
        self.glyph(c)[y as usize] & (0x10 >> x) != 0
    }

    /// Width in font pixels of `n` characters (no trailing gap).
    pub fn text_width(n: usize) -> u32 {
        if n == 0 {
            0
        } else {
            n as u32 * Self::ADVANCE - (Self::ADVANCE - Self::WIDTH)
        }
    }

    pub fn compile(src: &str) -> Result<Font, Report> {
        let mut report = Report::default();
        let file: FontFile = toml::from_str(src).map_err(|e| {
            report.error("", e.to_string());
            report.clone()
        })?;
        let mut glyphs = Vec::new();
        for code in Self::FIRST..=Self::LAST {
            let c = char::from_u32(code).expect("ASCII is valid");
            let Some(text) = file.glyphs.get(&c.to_string()) else {
                report.error("glyphs", format!("missing glyph {c:?}"));
                continue;
            };
            match glyph_rows(text) {
                Ok(rows) => glyphs.push(rows),
                Err(e) => report.error(format!("glyph {c:?}"), e),
            }
        }
        for key in file.glyphs.keys() {
            if !(key.chars().count() == 1
                && key
                    .chars()
                    .all(|c| (Self::FIRST..=Self::LAST).contains(&(c as u32))))
            {
                report.error(
                    "glyphs",
                    format!("{key:?} is not a printable ASCII character"),
                );
            }
        }
        if report.errors.is_empty() {
            Ok(Font { glyphs })
        } else {
            Err(report)
        }
    }
}

fn glyph_rows(text: &str) -> Result<[u8; 7], String> {
    let grid = parse_grid(text)?;
    if (grid.width, grid.height) != (Font::WIDTH as usize, Font::HEIGHT as usize) {
        return Err(format!(
            "glyphs are 5x7 (got {}x{})",
            grid.width, grid.height
        ));
    }
    let mut rows = [0u8; 7];
    for (i, g) in grid.glyphs.iter().enumerate() {
        match g {
            '#' => rows[i / 5] |= 0x10 >> (i % 5),
            '.' => {}
            other => return Err(format!("glyph pixels are '#' or '.', not {other:?}")),
        }
    }
    Ok(rows)
}

/// Which accent shade a crate pixel takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shade {
    Shadow,
    Base,
    Highlight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrateColor {
    Fixed(crate::Rgb),
    Accent(Shade),
}

/// A positioned grid of indices: 0 is transparent, `i` is `colors[i - 1]` of
/// the owning art.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedLayer {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The shared crate (SPEC §4.1–4.2). The nameplate is drawn in font-pixel
/// units: left cap, the middle column repeated once per text column, right
/// cap. Text starts after the left cap, vertically centered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateArt {
    pub colors: Vec<CrateColor>,
    pub back: IndexedLayer,
    pub front: IndexedLayer,
    pub plate_left: IndexedLayer,
    pub plate_mid: IndexedLayer,
    pub plate_right: IndexedLayer,
}

impl CrateArt {
    pub fn compile(src: &str, lib: &Library) -> Result<CrateArt, Report> {
        let mut report = Report::default();
        let file: CrateFile = toml::from_str(src).map_err(|e| {
            report.error("", e.to_string());
            report.clone()
        })?;
        let mut colors: Vec<CrateColor> = Vec::new();
        let mut glyphs: HashMap<char, u8> = HashMap::new();
        for (key, value) in &file.palette {
            let color = match value.as_str() {
                "clear" => None,
                "accent-shadow" => Some(CrateColor::Accent(Shade::Shadow)),
                "accent" => Some(CrateColor::Accent(Shade::Base)),
                "accent-highlight" => Some(CrateColor::Accent(Shade::Highlight)),
                name => match lib.palette.get(name) {
                    Some(c) => Some(CrateColor::Fixed(*c)),
                    None => {
                        report.error(
                            "palette",
                            format!("{name:?} is not a master palette color or accent shade"),
                        );
                        continue;
                    }
                },
            };
            match single_char(key) {
                Ok(c) => {
                    let index = color.map_or(0, |color| {
                        colors.push(color);
                        colors.len() as u8
                    });
                    glyphs.insert(c, index);
                }
                Err(e) => report.error("palette", e),
            }
        }

        let mut layers: BTreeMap<&str, IndexedLayer> = BTreeMap::new();
        for (name, layer) in &file.layers {
            let parsed = parse_grid(&layer.grid).and_then(|g| {
                let pixels = g
                    .glyphs
                    .iter()
                    .map(|c| {
                        glyphs
                            .get(c)
                            .copied()
                            .ok_or_else(|| format!("glyph {c:?} is not in the palette"))
                    })
                    .collect::<Result<_, _>>()?;
                let [x, y] = layer.at.unwrap_or([0, 0]);
                Ok(IndexedLayer {
                    x,
                    y,
                    width: g.width as u32,
                    height: g.height as u32,
                    pixels,
                })
            });
            match parsed {
                Ok(l) => {
                    layers.insert(name.as_str(), l);
                }
                Err(e) => report.error(format!("layer {name}"), e),
            }
        }

        let expect = [
            ("back", Some((0, 18, 32, 2))),
            ("front", Some((0, 20, 32, 12))),
            ("plate-left", None),
            ("plate-mid", None),
            ("plate-right", None),
        ];
        for name in file.layers.keys() {
            if !expect.iter().any(|(n, _)| n == name) {
                report.error(format!("layer {name}"), "unknown crate layer");
            }
        }
        for (name, geometry) in expect {
            match (layers.get(name), geometry) {
                (None, _) if report.errors.is_empty() => {
                    report.error("layers", format!("missing layer {name:?}"))
                }
                (Some(l), Some(g)) if (l.x, l.y, l.width, l.height) != g => {
                    report.error(
                        format!("layer {name}"),
                        format!("must be at [{}, {}] and {}x{}", g.0, g.1, g.2, g.3),
                    );
                }
                _ => {}
            }
        }
        if !report.errors.is_empty() {
            return Err(report);
        }
        let mut take = |n: &str| layers.remove(n).expect("checked above");
        let art = CrateArt {
            colors,
            back: take("back"),
            front: take("front"),
            plate_left: take("plate-left"),
            plate_mid: take("plate-mid"),
            plate_right: take("plate-right"),
        };
        let h = art.plate_mid.height;
        if art.plate_mid.width != 1
            || art.plate_left.height != h
            || art.plate_right.height != h
            || h < Font::HEIGHT + 2
        {
            report.error(
                "nameplate",
                "plate parts share one height (at least 9), and plate-mid is 1 column wide",
            );
            return Err(report);
        }
        Ok(art)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::lib_with;

    fn font_src(skip: char, extra: &str) -> String {
        let mut s = String::from("[glyphs]\n");
        for code in 0x20u8..=0x7e {
            let c = code as char;
            if c == skip {
                continue;
            }
            let key = match c {
                '"' => "'\"'".to_string(),
                '\\' => "'\\'".to_string(),
                _ => format!("\"{c}\""),
            };
            let rows = if c == 'I' {
                "#####\n..#..\n..#..\n..#..\n..#..\n..#..\n#####"
            } else {
                ".....\n.....\n.....\n.....\n.....\n.....\n#...#"
            };
            s.push_str(&format!("{key} = \"\"\"\n{rows}\n\"\"\"\n"));
        }
        s + extra
    }

    #[test]
    fn compiles_font() {
        let f = Font::compile(&font_src('\0', "")).unwrap();
        assert_eq!(f.glyphs.len(), 95);
        assert_eq!(f.glyph('I')[0], 0b11111);
        assert_eq!(f.glyph('I')[1], 0b00100);
        assert!(f.ink('I', 2, 3) && !f.ink('I', 0, 3));
        assert_eq!(f.glyph('é'), f.glyph('?'));
        assert_eq!(Font::text_width(0), 0);
        assert_eq!(Font::text_width(1), 5);
        assert_eq!(Font::text_width(3), 17);
    }

    #[test]
    fn font_errors() {
        let missing = Font::compile(&font_src('Q', "")).unwrap_err();
        assert!(missing.errors.iter().any(|d| d.message.contains("'Q'")));
        let extra = Font::compile(&font_src('\0', "\"é\" = \".....\"\n")).unwrap_err();
        assert!(extra.errors.iter().any(|d| d.message.contains("printable")));
    }

    const CRATE: &str = r#"
[palette]
"." = "clear"
"k" = "ink"
"2" = "accent"
"s" = "cream"
[layers.back]
at = [0, 18]
grid = """
kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk
k222222222222222222222222222222k
"""
[layers.front]
at = [0, 20]
grid = """
kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk
k222222222222222222222222222222k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k2ssssssssssssssssssssssssssss2k
k222222222222222222222222222222k
kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk
"""
[layers.plate-left]
grid = "k\nk\nk\nk\nk\nk\nk\nk\nk"
[layers.plate-mid]
grid = "k\ns\ns\ns\ns\ns\ns\ns\nk"
[layers.plate-right]
grid = "k\nk\nk\nk\nk\nk\nk\nk\nk"
"#;

    #[test]
    fn compiles_crate() {
        let lib = lib_with(&[], &[], &[]);
        let art = CrateArt::compile(CRATE, &lib).unwrap();
        assert_eq!((art.front.y, art.front.height), (20, 12));
        let accent_index = art
            .colors
            .iter()
            .position(|c| *c == CrateColor::Accent(Shade::Base))
            .unwrap() as u8
            + 1;
        assert_eq!(art.back.pixels[33], accent_index);
        assert_eq!(art.plate_mid.height, 9);
    }

    #[test]
    fn crate_errors() {
        let lib = lib_with(&[], &[], &[]);
        let moved =
            CrateArt::compile(&CRATE.replace("at = [0, 18]", "at = [0, 17]"), &lib).unwrap_err();
        assert!(
            moved
                .errors
                .iter()
                .any(|d| d.to_string().contains("layer back"))
        );
        let unknown = CrateArt::compile(
            &CRATE.replace("\"s\" = \"cream\"", "\"s\" = \"beige\""),
            &lib,
        )
        .unwrap_err();
        assert!(unknown.errors.iter().any(|d| d.message.contains("beige")));
    }
}
