//! Compiled mascot → the simple PNG form (SPEC §12.7): a `mascot.toml` plus
//! a `sprite.png` strip, the starting point `pawprint mascot export` gives
//! people for custom art.

use crate::compile::{MAX_ACTION_MS, MIN_MS};
use crate::{FRAME_SIZE, Mascot, RgbaImage, When};

/// The `mascot.toml` text and the sprite strip (every distinct frame, rest
/// first) for `m`. The result compiles as a user mascot that looks the same.
///
/// PNG actions have one `duration_ms` for all their frames, so a frame that
/// is held longer is listed several times (see [`quantize`]).
pub fn export_png(m: &Mascot) -> (String, RgbaImage) {
    let mut strip = RgbaImage::new(FRAME_SIZE * m.frames.len() as u32, FRAME_SIZE);
    for i in 0..m.frames.len() {
        let frame = m.frame_rgba(i);
        for y in 0..FRAME_SIZE {
            for x in 0..FRAME_SIZE {
                let at = i64::from(i as u32 * FRAME_SIZE + x);
                strip.put(at, i64::from(y), frame.pixel(x, y));
            }
        }
    }

    let s = |v: &str| toml::Value::from(v).to_string();
    let aliases: Vec<String> = m.aliases.iter().map(|a| s(a)).collect();
    let mut out = format!(
        "# {name}: a PAWPRINT mascot in the simple PNG form (SPEC §12.7).\n\
         # sprite.png is a strip of 32x32 frames, numbered from 0. Each action\n\
         # lists frame numbers; every frame shows for duration_ms (80-2000), so\n\
         # repeat a number to hold a frame. Edit the strip in any pixel editor,\n\
         # keep pixels fully opaque or fully transparent, then run\n\
         # `pawprint mascot check <this folder>`.\n\n\
         id = {id}\nname = {name_q}\ncategory = {category}\naliases = [{aliases}]\n\
         accent = {accent}\nfallback = {fallback}\nnocturnal = {nocturnal}\n\
         sprite = \"sprite.png\"\nrest = 0\n",
        name = m.name,
        id = s(&m.id),
        name_q = s(&m.name),
        category = s(&m.category),
        aliases = aliases.join(", "),
        accent = s(&m.accent),
        fallback = m.fallback,
        nocturnal = m.nocturnal,
    );
    for a in &m.actions {
        let ms: Vec<u32> = a.frames.iter().map(|f| u32::from(f.ms)).collect();
        let (unit, reps) = quantize(&ms);
        let frames: Vec<String> = a
            .frames
            .iter()
            .zip(reps)
            .flat_map(|(f, n)| std::iter::repeat_n(f.frame.to_string(), n as usize))
            .collect();
        out += &format!(
            "\n[[actions]]\nname = {}\nweight = {}\n",
            s(&a.name),
            a.weight
        );
        let when = match a.when {
            When::Any => None,
            When::Day => Some("day"),
            When::Night => Some("night"),
        };
        if let Some(w) = when {
            out += &format!("when = \"{w}\"\n");
        }
        out += &format!("duration_ms = {unit}\nframes = [{}]\n", frames.join(", "));
    }
    (out, strip)
}

/// Fit per-frame durations to one duration plus a repeat count per frame,
/// keeping the action within the length limit. The largest duration with
/// the smallest squared timing error wins, so frames that share a common
/// divisor of at least 80 ms come out exact.
fn quantize(ms: &[u32]) -> (u32, Vec<u32>) {
    let longest_unit = ms.iter().copied().min().unwrap_or(MIN_MS).max(MIN_MS);
    let fit = |unit: u32, round: bool| -> Vec<u32> {
        let half = if round { unit / 2 } else { 0 };
        ms.iter().map(|&m| ((m + half) / unit).max(1)).collect()
    };
    (MIN_MS..=longest_unit)
        .rev()
        .map(|unit| (unit, fit(unit, true)))
        .filter(|(unit, reps)| reps.iter().sum::<u32>() * unit <= MAX_ACTION_MS)
        .min_by_key(|(unit, reps)| {
            ms.iter()
                .zip(reps)
                .map(|(&m, &n)| m.abs_diff(n * unit).pow(2))
                .sum::<u32>()
        })
        // Rounding up can overrun the limit; rounding down never does.
        .unwrap_or_else(|| (MIN_MS, fit(MIN_MS, false)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::lib_with;
    use crate::{Action, FrameRef, Origin, Rgb, compile};

    /// Three frames, two colors, an uneven action and a night action.
    fn sample_mascot() -> Mascot {
        let frame = |row: usize, color: u8| {
            let mut f = vec![0u8; 32 * 32];
            f[row * 32..row * 32 + 32].fill(color);
            f
        };
        let r = |frame: u16, ms: u16| FrameRef { frame, ms };
        Mascot {
            id: "my-owl".into(),
            name: "My \"Night\" Owl".into(),
            category: "birds".into(),
            aliases: vec!["hooter".into(), "barn owl".into()],
            accent: "#336699".into(),
            fallback: false,
            nocturnal: true,
            base: None,
            colors: vec![Rgb(10, 20, 30), Rgb(200, 100, 50)],
            frames: vec![frame(3, 1), frame(4, 2), frame(5, 1)],
            actions: vec![
                Action {
                    name: "blink".into(),
                    weight: 6,
                    when: When::Any,
                    template: Some("blink".into()),
                    frames: vec![r(1, 140), r(0, 200), r(2, 900)],
                },
                Action {
                    name: "spin".into(),
                    weight: 1,
                    when: When::Night,
                    template: None,
                    frames: vec![r(2, 200), r(1, 400)],
                },
            ],
        }
    }

    #[test]
    fn quantize_is_exact_with_a_common_divisor() {
        assert_eq!(quantize(&[200, 200, 600]), (200, vec![1, 1, 3]));
        assert_eq!(quantize(&[160, 480]), (160, vec![1, 3]));
        assert_eq!(quantize(&[90]), (90, vec![1]));
    }

    #[test]
    fn quantize_approximates_and_respects_the_limit() {
        let (unit, reps) = quantize(&[140, 200, 900]);
        let worst = [140u32, 200, 900]
            .iter()
            .zip(&reps)
            .map(|(&m, &n)| m.abs_diff(n * unit))
            .max()
            .unwrap();
        assert!(worst <= 30, "unit {unit}, reps {reps:?}, off by {worst} ms");
        let (unit, reps) = quantize(&[1990, 1990]);
        assert!(reps.iter().sum::<u32>() * unit <= MAX_ACTION_MS);
    }

    #[test]
    fn export_round_trips_through_the_png_compiler() {
        let m = sample_mascot();
        let (toml, strip) = export_png(&m);
        let png = strip.encode_png();
        let lib = lib_with(&[], &[], &[]);
        let c = compile(&toml, Some(&m.id), &lib, Origin::User, &|name| {
            assert_eq!(name, "sprite.png");
            Ok(png.clone())
        });
        assert!(c.report.is_clean(), "{:?}\n{toml}", c.report);
        let back = c.mascot.unwrap();
        assert_eq!(
            (
                &back.id,
                &back.name,
                &back.category,
                &back.aliases,
                &back.accent
            ),
            (&m.id, &m.name, &m.category, &m.aliases, &m.accent)
        );
        assert_eq!((back.fallback, back.nocturnal), (m.fallback, m.nocturnal));
        assert_eq!(back.frames.len(), m.frames.len());
        for i in 0..m.frames.len() {
            assert_eq!(back.frame_rgba(i), m.frame_rgba(i), "frame {i}");
        }
        for (a, b) in m.actions.iter().zip(&back.actions) {
            assert_eq!((&a.name, a.weight, a.when), (&b.name, b.weight, b.when));
            assert_eq!(b.template, None);
        }
    }
}
