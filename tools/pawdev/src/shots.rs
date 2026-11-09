//! `make settings-shots`: every page of the settings window (SPEC §8.7) as
//! a PNG, for review, and the README's screenshot.

use std::path::Path;
use std::sync::Arc;

use pawprint_core::catalog::{Catalog, bundle};
use pawprint_core::geom::PhysicalRect;
use pawprint_core::settings::{App, Effect, Inputs, Page};
use pawprint_core::state::State;
use pawprint_core::surface::DisplayInfo;
use pawprint_sprite::RgbaImage;

/// A fleet config with a little of everything, for a host named `owl`.
const SAMPLE: &str = r#"# The fleet's config: copied to every machine.
accent = "teal"

[hosts."prod-*"]
accent = "cranberry"
label_case = "upper"

[hosts.buildbox]
mascot = "capybara"

[hosts.owl]
size = 96
"#;

pub fn settings_shots(root: &Path) -> Result<(), String> {
    let out = root.join("out/settings");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let inputs = Inputs {
        hostname: "owl".into(),
        text: SAMPLE.into(),
        state: State::default(),
        path: "~/.config/pawprint/config.toml".into(),
    };
    let mut app = App::new(Arc::new(Catalog::bundled().clone()), bundle(), inputs);
    let display = DisplayInfo {
        name: "DP-1".into(),
        bounds: PhysicalRect::new(0, 0, 1920, 1080),
        work_area: PhysicalRect::new(0, 0, 1920, 1040),
        scale: 1.0,
        primary: true,
    };
    app.start(vec![display]);
    for page in App::pages() {
        let frame = app.set_page(page).into_iter().find_map(|e| match e {
            Effect::Present(f) => Some(f),
            _ => None,
        });
        let Some(frame) = frame else {
            return Err(format!("{page:?} drew nothing"));
        };
        // Frames are premultiplied BGRA; every pixel here is opaque.
        let mut img = RgbaImage::new(frame.width, frame.height);
        for (dst, src) in img.data.chunks_mut(4).zip(frame.bgra.chunks(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        let name = format!("{page:?}").to_lowercase();
        let path = out.join(format!("{name}.png"));
        std::fs::write(&path, img.encode_png()).map_err(|e| format!("{}: {e}", path.display()))?;
        if page == Page::Mascot {
            let doc = root.join("docs/settings.png");
            std::fs::write(&doc, img.encode_png())
                .map_err(|e| format!("{}: {e}", doc.display()))?;
            println!("wrote {}", doc.display());
        }
        println!("wrote {}", path.display());
    }
    Ok(())
}
