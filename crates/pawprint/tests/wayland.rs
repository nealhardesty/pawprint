//! The Wayland backend against headless sway (SPEC §11.4): layer-shell
//! placement, click-through, move mode with the implicit grab (F7), panels,
//! and output changes. Needs `sway`, `swaybg`, `grim`, `foot`, and `wtype`;
//! run with `make test-wayland`.

#![cfg(target_os = "linux")]

mod common;

use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use common::{Env, Image, Proc, assert_badge_at, wait_for};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

const BACKGROUND: [u8; 3] = [0x33, 0x66, 0x99];
const BTN_LEFT: u32 = 0x110;

/// A headless sway session.
struct Sway {
    socket: PathBuf,
    _sway: Proc,
}

impl Sway {
    fn start(env: &mut Env, extra_config: &str) -> Sway {
        let config = env.dir.join("sway.conf");
        std::fs::write(
            &config,
            format!(
                "output * mode 1280x720 bg #336699 solid_color\nfocus_follows_mouse no\n\
                 # Keep the cursor out of screenshots.\nseat * hide_cursor 100\n{extra_config}\n"
            ),
        )
        .unwrap();
        env.set("WLR_BACKENDS", "headless");
        env.set("WLR_LIBINPUT_NO_DEVICES", "1");
        env.set("WLR_RENDERER", "pixman");
        let sway = env.spawn("sway", &["-c", config.to_str().unwrap()]);
        let runtime = env.runtime();
        let find = |prefix: &str, suffix: &str| {
            std::fs::read_dir(&runtime)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .find(|p| {
                    let n = p.file_name().unwrap().to_string_lossy().into_owned();
                    n.starts_with(prefix) && n.ends_with(suffix)
                })
        };
        let socket = wait_for("the sway socket", || {
            find("wayland-", "").filter(|p| p.extension().is_none())
        });
        let ipc = wait_for("the sway IPC socket", || find("sway-ipc.", ".sock"));
        env.set(
            "WAYLAND_DISPLAY",
            socket.file_name().unwrap().to_string_lossy(),
        );
        env.set("SWAYSOCK", ipc.to_string_lossy());
        let sway = Sway {
            socket,
            _sway: sway,
        };
        // swaybg paints the background asynchronously.
        wait_for("the background", || {
            (sway.grab(env).px(640, 360)[..3] == BACKGROUND).then_some(())
        });
        sway
    }

    fn grab(&self, env: &Env) -> Image {
        let path = env.dir.join("screen.png");
        let out = env.command("grim").arg(&path).output().unwrap();
        assert!(
            out.status.success(),
            "grim: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Image::load(&path)
    }

    fn msg(&self, env: &Env, args: &[&str]) -> String {
        let out = env.command("swaymsg").args(args).output().unwrap();
        assert!(
            out.status.success(),
            "swaymsg {args:?}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The app_id of the focused window, if any.
    fn focused(&self, env: &Env) -> Option<String> {
        let tree = self.msg(env, &["-t", "get_tree"]);
        // The focused node is the innermost `"focused": true`; its app_id
        // follows it in swaymsg's JSON.
        let at = tree.rfind("\"focused\": true")?;
        let rest = &tree[at..];
        let id = rest.find("\"app_id\": \"")? + 11;
        Some(rest[id..id + rest[id..].find('"')?].to_string())
    }

    fn windows(&self, env: &Env) -> usize {
        self.msg(env, &["-t", "get_tree"])
            .matches("\"app_id\": \"")
            .count()
    }
}

/// A virtual pointer, so the seat has a pointer and the test can drive it.
struct Pointer {
    queue: EventQueue<PointerState>,
    state: PointerState,
    ptr: ZwlrVirtualPointerV1,
    time: u32,
}

struct PointerState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PointerState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(PointerState: ignore wl_seat::WlSeat);
delegate_noop!(PointerState: ZwlrVirtualPointerManagerV1);
delegate_noop!(PointerState: ZwlrVirtualPointerV1);

impl Pointer {
    fn new(sway: &Sway) -> Pointer {
        let conn = Connection::from_socket(UnixStream::connect(&sway.socket).unwrap()).unwrap();
        let (globals, mut queue) = registry_queue_init::<PointerState>(&conn).unwrap();
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
        let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).unwrap();
        let ptr = manager.create_virtual_pointer(Some(&seat), &qh, ());
        let mut state = PointerState;
        queue.roundtrip(&mut state).unwrap();
        Pointer {
            queue,
            state,
            ptr,
            time: 0,
        }
    }

    fn sync(&mut self) {
        self.ptr.frame();
        self.queue.roundtrip(&mut self.state).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(40));
    }

    fn to(&mut self, x: u32, y: u32) {
        self.time += 10;
        self.ptr.motion_absolute(self.time, x, y, 1280, 720);
        self.sync();
    }

    fn button(&mut self, pressed: bool) {
        self.time += 10;
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        self.ptr.button(self.time, BTN_LEFT, state);
        self.sync();
    }

    fn click(&mut self, x: u32, y: u32) {
        self.to(x, y);
        self.button(true);
        self.button(false);
    }
}

fn badge_session(tag: &str, extra_config: &str) -> (Env, Sway, Pointer, Proc) {
    let mut env = Env::new(tag);
    let sway = Sway::start(&mut env, extra_config);
    let pointer = Pointer::new(&sway);
    let badge = env.run_badge();
    wait_for("the badge to answer", || env.badge_rect());
    (env, sway, pointer, badge)
}

#[test]
#[ignore = "needs sway, swaybg, grim, foot; run `make test-wayland`"]
fn layer_surface_draws_top_right_and_passes_clicks() {
    let (env, sway, mut pointer, _badge) = badge_session("wl-place", "");
    env.wait_rect([1200, 16, 64, 64]);
    let expected = env.expected(64);
    wait_for("the badge on screen", || {
        (sway.grab(&env).px(1232, 60) != [0x33, 0x66, 0x99, 255]).then_some(())
    });
    assert_badge_at(&sway.grab(&env), &expected, 1200, 16, BACKGROUND);
    let status = env.ok(&["status"]);
    assert!(
        status.contains("backend     wayland") && status.contains("HEADLESS-1"),
        "{status}"
    );

    // Two tiled windows: "left" and "right"; the badge sits over "right".
    let _left = env.spawn("foot", &["--app-id", "left"]);
    wait_for("one window", || (sway.windows(&env) == 1).then_some(()));
    let _right = env.spawn("foot", &["--app-id", "right"]);
    wait_for("two windows", || (sway.windows(&env) == 2).then_some(()));
    sway.msg(&env, &["[app_id=left]", "focus"]);
    assert_eq!(sway.focused(&env).as_deref(), Some("left"));

    // Click-through (F4): clicking the badge focuses the window under it.
    pointer.click(1232, 60);
    assert_eq!(
        sway.focused(&env).as_deref(),
        Some("right"),
        "click passes through the badge"
    );

    // In move mode the badge takes the click.
    sway.msg(&env, &["[app_id=left]", "focus"]);
    env.ok(&["move"]);
    pointer.click(1232, 60);
    assert_ne!(
        sway.focused(&env).as_deref(),
        Some("right"),
        "move mode keeps the click"
    );
    wait_for("move mode to end", || {
        (!env.ok(&["status"]).contains("move mode   on")).then_some(())
    });
    sway.msg(&env, &["[app_id=left]", "focus"]);
    pointer.click(1232, 60);
    assert_eq!(
        sway.focused(&env).as_deref(),
        Some("right"),
        "click-through again afterwards"
    );
}

#[test]
#[ignore = "needs sway, swaybg, grim, wtype; run `make test-wayland`"]
fn move_mode_drags_with_the_implicit_grab_and_nudges() {
    let (env, sway, mut pointer, _badge) = badge_session("wl-move", "");
    env.wait_rect([1200, 16, 64, 64]);
    let expected = env.expected(64);

    // F7: press on the badge, then move far outside it before releasing.
    env.ok(&["move"]);
    pointer.to(1232, 48);
    pointer.button(true);
    for (x, y) in [(1100, 120), (800, 300), (500, 380), (300, 400)] {
        pointer.to(x, y);
    }
    pointer.button(false);
    env.wait_rect([268, 368, 64, 64]);
    let state = env.state();
    assert!(
        state.contains("anchor = \"left\"") && state.contains("offset = [268, 40]"),
        "{state}"
    );
    wait_for("the badge at its new spot", || {
        let screen = sway.grab(&env);
        (screen.px(268 + 32, 368 + 44)[..3] == expected.px(32, 44)[..3]).then_some(())
    });
    assert_badge_at(&sway.grab(&env), &expected, 268, 368, BACKGROUND);

    // Arrow keys: exclusive keyboard in move mode, no click needed.
    env.ok(&["move"]);
    // Headless sway has no keyboard until wtype adds its virtual one, so
    // wait a moment for the badge to bind it before typing.
    let key = |k: &str| {
        let out = env
            .command("wtype")
            .args(["-s", "300", "-k", k, "-s", "100"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "wtype: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    key("Left");
    key("Left");
    env.wait_rect([266, 368, 64, 64]);
    key("Return");
    wait_for("Enter to save", || {
        env.state().contains("offset = [266, 40]").then_some(())
    });
    env.ok(&["move"]);
    key("Right");
    env.wait_rect([267, 368, 64, 64]);
    key("Escape");
    env.wait_rect([266, 368, 64, 64]);
}

#[test]
#[ignore = "needs sway, swaybg, grim; run `make test-wayland`"]
fn panels_shift_the_badge_into_the_usable_area() {
    // swaybar takes an exclusive zone at the top (F3).
    let (env, sway, _pointer, _badge) = badge_session(
        "wl-panel",
        "bar {\n  position top\n  status_command echo pawprint\n}",
    );
    let doctor = wait_for("a usable area smaller than the output", || {
        let d = env.ok(&["doctor"]);
        (!d.contains("usable 1280x720")).then_some(d)
    });
    let line = doctor
        .lines()
        .find(|l| l.contains("HEADLESS-1"))
        .unwrap()
        .to_string();
    let usable_h: i32 = line
        .split("usable 1280x")
        .nth(1)
        .unwrap()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let bar = 720 - usable_h;
    assert!(bar > 0, "{line}");
    let expected = env.expected(64);
    wait_for("the badge under the bar", || {
        let s = sway.grab(&env);
        (s.px(1232, (bar + 60) as u32)[..3] == expected.px(32, 44)[..3]).then_some(())
    });
    assert_badge_at(&sway.grab(&env), &expected, 1200, 16 + bar, BACKGROUND);
}

#[test]
#[ignore = "needs sway, swaybg, grim; run `make test-wayland`"]
fn output_mode_and_scale_changes_recreate_and_stay_crisp() {
    let (env, sway, _pointer, _badge) = badge_session("wl-output", "");
    env.wait_rect([1200, 16, 64, 64]);
    let expected = env.expected(64);
    let palette = expected.opaque_colors();
    let opaque = expected.opaque_count() as f64;
    // Rows above the owl's ears are transparent: the visible box starts lower.
    let top = expected.opaque_bbox()[1] / 2; // in art pixels (P = 2 at 64)
    // Find the badge by its exact palette colors in the top-right quarter
    // (clear of the pointer). Blurred scaling would leave mixed colors, so
    // nearly every opaque pixel must be found, scaled by (P / 2)^2.
    let quarter = [512, 0, 512, 384];
    let shows = |x: i32, y: i32, px: i32, p: i32| {
        let want = [x, y + top * p, px, px - top * p];
        let mut seen = None;
        let start = std::time::Instant::now();
        while seen != Some(want) {
            assert!(
                start.elapsed().as_secs() < 10,
                "badge pixels at {seen:?}, want {want:?}"
            );
            seen = sway.grab(&env).bbox_of(quarter, &palette);
        }
        let exact = sway.grab(&env).count_of(want, &palette) as f64;
        let ideal = opaque * f64::from(p * p) / 4.0;
        assert!(
            (exact - ideal).abs() / ideal < 0.03,
            "{exact} exact pixels, want about {ideal}: not crisp"
        );
    };

    sway.msg(&env, &["output", "HEADLESS-1", "mode", "1024x768"]);
    shows(1024 - 80, 16, 64, 2);

    sway.msg(&env, &["output", "HEADLESS-1", "scale", "2"]);
    shows(1024 - 128 - 32, 32, 128, 4);

    // At 1.5 the core sizes the badge 96 px (P = 3). Headless sway's pixman
    // renderer filters the whole output at fractional scales (even swaybg's
    // solid color comes out uneven), so only the size and place are checked
    // here; crispness at fractional scales is an owner check on Plasma.
    sway.msg(&env, &["output", "HEADLESS-1", "scale", "1.5"]);
    let found = wait_for("a 96 px badge", || {
        sway.grab(&env)
            .bbox_of(quarter, &palette)
            .filter(|b| b[2] == 96)
    });
    // 1024 / 1.5 = 682.67 logical px; sway uses 682, so the right edge is 1023.
    assert_eq!(found[0], 1023 - 96 - 24, "{found:?}");
    let exact = sway.grab(&env).count_of([found[0], 24, 96, 96], &palette);
    eprintln!(
        "scale 1.5: {exact} of about {} opaque pixels exact",
        opaque * 9.0 / 4.0
    );
}
