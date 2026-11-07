//! The X11 backend against a real X server (SPEC §11.3): Xvfb alone, with a
//! compositing manager, and with a window manager. Needs `Xvfb`,
//! `xcompmgr`, and `openbox`; run with `make test-x11`.

#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use common::{Env, Image, Proc, assert_badge_at, wait_for};
use x11rb::connection::Connection;
use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::xproto::*;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const W: u16 = 1280;
const H: u16 = 720;
const BACKGROUND: [u8; 3] = [0x33, 0x66, 0x99];

/// An Xvfb server and a test client connection to it.
struct X {
    conn: RustConnection,
    root: Window,
    _server: Proc,
}

impl X {
    /// Start Xvfb on a free display. It listens on TCP so the test works
    /// where `/tmp/.X11-unix` is read-only (x11rb can't use abstract sockets).
    fn start(env: &mut Env) -> X {
        let n = (150..250)
            .find(|n| !std::path::Path::new(&format!("/tmp/.X{n}-lock")).exists())
            .expect("a free display number");
        let server = env.spawn(
            "Xvfb",
            &[
                &format!(":{n}"),
                "-screen",
                "0",
                &format!("{W}x{H}x24"),
                "-listen",
                "tcp",
                "-ac",
                "-nolisten",
                "unix",
            ],
        );
        let display = format!("127.0.0.1:{n}");
        env.set("DISPLAY", &display);
        let (conn, screen) = wait_for("Xvfb", || x11rb::connect(Some(&display)).ok());
        let root = conn.setup().roots[screen].root;
        X {
            conn,
            root,
            _server: server,
        }
    }

    /// A full-screen window in `BACKGROUND` that records clicks.
    fn background(&self, managed: bool, rect: (i16, i16, u16, u16), color: u32) -> Window {
        let win = self.conn.generate_id().unwrap();
        let aux = CreateWindowAux::new()
            .background_pixel(color)
            .override_redirect(u32::from(!managed))
            .event_mask(EventMask::BUTTON_PRESS);
        self.conn
            .create_window(
                24,
                win,
                self.root,
                rect.0,
                rect.1,
                rect.2,
                rect.3,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &aux,
            )
            .unwrap();
        let spec = WmSizeHintsSpecification::UserSpecified;
        WmSizeHints {
            position: Some((spec, rect.0.into(), rect.1.into())),
            size: Some((spec, rect.2.into(), rect.3.into())),
            ..WmSizeHints::new()
        }
        .set_normal_hints(&self.conn, win)
        .unwrap();
        self.conn.map_window(win).unwrap();
        self.conn.flush().unwrap();
        win
    }

    fn capture(&self) -> Image {
        let img = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, 0, 0, W, H, !0)
            .unwrap()
            .reply()
            .unwrap();
        let rgba = img
            .data
            .chunks(4)
            .flat_map(|p| [p[2], p[1], p[0], 255])
            .collect();
        Image {
            width: W.into(),
            height: H.into(),
            rgba,
        }
    }

    /// The badge window, found by WM_CLASS anywhere in the tree.
    fn badge(&self) -> Option<Window> {
        let mut stack = vec![self.root];
        while let Some(w) = stack.pop() {
            // Windows can vanish mid-walk (the badge is rebuilt when the
            // compositor changes); skip them.
            let class = self
                .conn
                .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
                .ok()
                .and_then(|c| c.reply().ok());
            if class.is_some_and(|c| c.value.starts_with(b"pawprint\0")) {
                return Some(w);
            }
            if let Some(tree) = self.conn.query_tree(w).ok().and_then(|c| c.reply().ok()) {
                stack.extend(tree.children);
            }
        }
        None
    }

    fn badge_mapped_within(&self, limit: Duration) -> Option<Window> {
        let start = Instant::now();
        while start.elapsed() < limit {
            if let Some(w) = self.badge()
                && self
                    .conn
                    .get_window_attributes(w)
                    .ok()?
                    .reply()
                    .ok()?
                    .map_state
                    == MapState::VIEWABLE
            {
                return Some(w);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn mapped_badge(&self) -> Window {
        wait_for("a mapped badge window", || {
            let w = self.badge()?;
            let attrs = self.conn.get_window_attributes(w).ok()?.reply().ok()?;
            (attrs.map_state == MapState::VIEWABLE).then_some(w)
        })
    }

    fn atom(&self, name: &str) -> Atom {
        self.conn
            .intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    }

    fn atoms(&self, w: Window, prop: &str) -> Vec<Atom> {
        let reply = self
            .conn
            .get_property(false, w, self.atom(prop), AtomEnum::ATOM, 0, 64)
            .unwrap()
            .reply()
            .unwrap();
        reply.value32().map(Iterator::collect).unwrap_or_default()
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0)
            .unwrap();
        self.conn.sync().unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }

    fn pointer_to(&self, x: i16, y: i16) {
        self.fake(MOTION_NOTIFY_EVENT, 0, x, y);
    }

    fn click(&self, x: i16, y: i16) {
        self.pointer_to(x, y);
        self.fake(BUTTON_PRESS_EVENT, 1, x, y);
        self.fake(BUTTON_RELEASE_EVENT, 1, x, y);
    }

    fn drag(&self, from: (i16, i16), to: (i16, i16)) {
        self.pointer_to(from.0, from.1);
        self.fake(BUTTON_PRESS_EVENT, 1, from.0, from.1);
        self.pointer_to((from.0 + to.0) / 2, (from.1 + to.1) / 2);
        self.pointer_to(to.0, to.1);
        self.fake(BUTTON_RELEASE_EVENT, 1, to.0, to.1);
    }

    fn key(&self, keysym: u32, shift: bool) {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .unwrap()
            .reply()
            .unwrap();
        let per = usize::from(map.keysyms_per_keycode);
        let code = |sym: u32| {
            min + map
                .keysyms
                .chunks(per)
                .position(|s| s[0] == sym)
                .expect("keysym in map") as u8
        };
        let shift_code = code(0xffe1);
        if shift {
            self.fake(KEY_PRESS_EVENT, shift_code, 0, 0);
        }
        self.fake(KEY_PRESS_EVENT, code(keysym), 0, 0);
        self.fake(KEY_RELEASE_EVENT, code(keysym), 0, 0);
        if shift {
            self.fake(KEY_RELEASE_EVENT, shift_code, 0, 0);
        }
    }

    /// Did `win` get a button press within half a second?
    fn clicked(&self, win: Window) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {
            while let Some(e) = self.conn.poll_for_event().unwrap() {
                if let x11rb::protocol::Event::ButtonPress(b) = e
                    && b.event == win
                {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// The window's depth, or `None` if it is gone (being recreated).
    fn depth(&self, w: Window) -> Option<u8> {
        Some(self.conn.get_geometry(w).ok()?.reply().ok()?.depth)
    }

    fn set_root_property(&self, name: &str, kind: impl Into<Atom>, data: Prop) {
        let atom = self.atom(name);
        match data {
            Prop::Cardinals(v) => {
                self.conn
                    .change_property32(PropMode::REPLACE, self.root, atom, kind, &v)
            }
            Prop::Text(t) => {
                self.conn
                    .change_property8(PropMode::REPLACE, self.root, atom, kind, t.as_bytes())
            }
        }
        .unwrap();
        self.conn.flush().unwrap();
    }
}

enum Prop {
    Cardinals(Vec<u32>),
    Text(String),
}

const BG_PIXEL: u32 = 0x336699;
/// Inside the label strip of a 64 px badge at the default top-right spot.
const LABEL: (i16, i16) = (1232, 60);

#[test]
#[ignore = "needs Xvfb; run `make test-x11`"]
fn bare_server_shaped_override_redirect_and_move_mode() {
    let mut env = Env::new("x11-bare");
    let x = X::start(&mut env);
    let bg = x.background(false, (0, 0, W, H), BG_PIXEL);
    let _badge = env.run_badge();
    let win = x.mapped_badge();
    env.wait_rect([1200, 16, 64, 64]);
    let expected = env.expected(64);

    // No WM: override-redirect. No compositor: the default visual, shaped.
    let attrs = x.conn.get_window_attributes(win).unwrap().reply().unwrap();
    assert!(
        attrs.override_redirect,
        "no window manager → override-redirect"
    );
    assert_eq!(x.depth(win), Some(24), "no compositor → default visual");
    assert_eq!(
        x.atoms(win, "_NET_WM_WINDOW_TYPE"),
        vec![x.atom("_NET_WM_WINDOW_TYPE_DOCK")]
    );
    let state = x.atoms(win, "_NET_WM_STATE");
    for s in [
        "_NET_WM_STATE_ABOVE",
        "_NET_WM_STATE_STICKY",
        "_NET_WM_STATE_SKIP_TASKBAR",
        "_NET_WM_STATE_SKIP_PAGER",
    ] {
        assert!(state.contains(&x.atom(s)), "{s}");
    }
    // Transparent through the bounding shape, pixel for pixel.
    assert_badge_at(&x.capture(), &expected, 1200, 16, BACKGROUND);

    // Click-through: a click on the label lands on the window underneath.
    x.click(LABEL.0, LABEL.1);
    assert!(x.clicked(bg), "click passes through the badge");

    // Move mode takes the pointer: drag from the badge to (300, 400).
    env.ok(&["move"]);
    x.drag((1232, 48), (300, 400));
    assert!(!x.clicked(bg), "move mode keeps the click");
    env.wait_rect([268, 368, 64, 64]);
    let state = env.state();
    assert!(
        state.contains("anchor = \"left\"") && state.contains("offset = [268, 40]"),
        "{state}"
    );
    assert_badge_at(&x.capture(), &expected, 268, 368, BACKGROUND);
    x.click(300, 400);
    assert!(x.clicked(bg), "click-through again after the drag");

    // Keyboard: Left nudges 1 px, Shift+Up 10 px, Enter saves; Esc cancels.
    env.ok(&["move"]);
    x.key(0xff51, false);
    x.key(0xff52, true);
    x.key(0xff0d, false);
    env.wait_rect([267, 358, 64, 64]);
    env.ok(&["move"]);
    x.key(0xff53, false);
    env.wait_rect([268, 358, 64, 64]);
    x.key(0xff1b, false);
    env.wait_rect([267, 358, 64, 64]);

    // Display changes: a top panel in the work area, then 2x via Xft.dpi.
    env.ok(&["reset-position"]);
    env.wait_rect([1200, 16, 64, 64]);
    x.set_root_property(
        "_NET_WORKAREA",
        AtomEnum::CARDINAL,
        Prop::Cardinals(vec![0, 40, 1280, 680]),
    );
    env.wait_rect([1200, 56, 64, 64]);
    x.set_root_property(
        "RESOURCE_MANAGER",
        AtomEnum::STRING,
        Prop::Text("Xft.dpi:\t192\n".into()),
    );
    env.wait_rect([1280 - 128 - 32, 40 + 32, 128, 128]);

    env.ok(&["quit"]);
    wait_for("the badge to exit", || x.badge().is_none().then_some(()));
}

#[test]
#[ignore = "needs Xvfb and xcompmgr; run `make test-x11`"]
fn compositor_switches_to_argb_and_back() {
    let mut env = Env::new("x11-cm");
    let x = X::start(&mut env);
    let bg = x.background(false, (0, 0, W, H), BG_PIXEL);
    let badge = env.run_badge();
    assert_eq!(x.depth(x.mapped_badge()), Some(24));

    // A compositing manager appears: the window is rebuilt with ARGB.
    let compositor = env.spawn("xcompmgr", &[]);
    let win = wait_for("a 32-bit badge window", || {
        let w = x.badge()?;
        (x.depth(w) == Some(32)
            && x.conn
                .get_window_attributes(w)
                .ok()?
                .reply()
                .ok()?
                .map_state
                == MapState::VIEWABLE)
            .then_some(w)
    });
    env.wait_rect([1200, 16, 64, 64]);
    let expected = env.expected(64);
    wait_for("composited pixels", || {
        let screen = x.capture();
        (screen.px(1232, 60)[..3] == expected.px(32, 44)[..3]).then_some(())
    });
    assert_badge_at(&x.capture(), &expected, 1200, 16, BACKGROUND);
    x.click(LABEL.0, LABEL.1);
    assert!(x.clicked(bg), "click-through with ARGB");
    assert!(env.log().contains("32-bit ARGB"), "{}", env.log());

    // And disappears: back to the shaped default visual.
    drop(compositor);
    wait_for("a 24-bit badge window", || {
        x.badge().filter(|&w| x.depth(w) == Some(24) && w != win)
    });
    assert_badge_at(&x.capture(), &expected, 1200, 16, BACKGROUND);
    drop(badge);
}

#[test]
#[ignore = "needs Xvfb and openbox; run `make test-x11`"]
fn window_manager_keeps_the_dock_above_and_in_place() {
    let mut env = Env::new("x11-wm");
    let x = X::start(&mut env);
    // An override-redirect backdrop could cover managed windows, so paint
    // the root instead.
    x.conn
        .change_window_attributes(
            x.root,
            &ChangeWindowAttributesAux::new().background_pixel(BG_PIXEL),
        )
        .unwrap();
    x.conn.clear_area(false, x.root, 0, 0, 0, 0).unwrap();
    let mut wm = env.spawn("openbox", &["--sm-disable"]);
    // Openbox announces itself (_NET_SUPPORTING_WM_CHECK) before it has
    // finished starting; _NET_WORKAREA comes once it manages the screen.
    wait_for("openbox", || {
        let set = |name: &str| {
            let r = x
                .conn
                .get_property(false, x.root, x.atom(name), AtomEnum::ANY, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            (r.value_len > 0).then_some(())
        };
        set("_NET_SUPPORTING_WM_CHECK")?;
        set("_NET_WORKAREA")
    });
    // Low enough to sit on the client area of the window below, under its
    // title bar.
    env.write_config("offset = [16, 100]");
    let _badge = env.run_badge();
    let win = x.badge_mapped_within(std::time::Duration::from_secs(10)).unwrap_or_else(|| {
        let w = x.badge();
        let state = w.and_then(|w| x.conn.get_window_attributes(w).ok()?.reply().ok()).map(|a| a.map_state);
        let parent = w.and_then(|w| x.conn.query_tree(w).ok()?.reply().ok()).map(|t| t.parent);
        panic!("badge {w:?} not viewable: state {state:?}, parent {parent:?}, openbox exited: {:?}", wm.exited())
    });
    let attrs = x.conn.get_window_attributes(win).unwrap().reply().unwrap();
    assert!(
        !attrs.override_redirect,
        "a WM is running → managed DOCK window"
    );
    env.wait_rect([1200, 100, 64, 64]);

    // A normal window mapped later, right under the badge, stays below it
    // and gets the clicks.
    let red = x.background(true, (900, 0, 380, 300), 0x993333);
    let expected = env.expected(64);
    wait_for("the window under the badge", || {
        (x.capture().px(1000, 250)[..3] == [0x99, 0x33, 0x33]).then_some(())
    });
    let geo = x.conn.get_geometry(win).unwrap().reply().unwrap();
    let pos = x
        .conn
        .translate_coordinates(win, x.root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(
        (pos.dst_x, pos.dst_y, geo.width, geo.height),
        (1200, 100, 64, 64),
        "the WM keeps the position"
    );
    assert_badge_at(&x.capture(), &expected, 1200, 100, [0x99, 0x33, 0x33]);
    let _ = x.clicked(red); // drain
    x.click(1232, 144);
    assert!(x.clicked(red), "click passes through to the managed window");

    env.ok(&["move"]);
    x.drag((1232, 132), (300, 400));
    env.wait_rect([268, 368, 64, 64]);
}

#[test]
#[ignore = "needs Xvfb and xcompmgr; run `make test-x11`"]
fn gnome_profile_through_xwayland() {
    // GNOME on Wayland without layer-shell (SPEC §3.2 step 2). The Wayland
    // display is unreachable here, so layer-shell is "missing".
    let mut env = Env::new("x11-gnome");
    let x = X::start(&mut env);
    x.background(false, (0, 0, W, H), BG_PIXEL);
    let _compositor = env.spawn("xcompmgr", &[]);
    env.set("WAYLAND_DISPLAY", "pawprint-test-none");
    env.set("XDG_CURRENT_DESKTOP", "ubuntu:GNOME");
    let _badge = env.run_badge();
    let win = x.mapped_badge();
    assert_eq!(x.depth(win), Some(32), "GNOME composites, so ARGB");
    let status = env.ok(&["status"]);
    assert!(status.contains("x11 (gnome profile)"), "{status}");
    assert!(
        status.contains("wayland-pawprint-test-none"),
        "keyed on WAYLAND_DISPLAY: {status}"
    );
}

#[test]
#[ignore = "needs Xvfb; run `make test-x11`"]
fn single_instance_and_detached_start() {
    let mut env = Env::new("x11-instance");
    let x = X::start(&mut env);
    let mut first = env.run_badge();
    x.mapped_badge();
    env.wait_rect([1200, 16, 64, 64]);

    // A second `run` sees the first and exits 0; bare `pawprint` says so too.
    let second = env.pawprint(&["run"]);
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("already running"));
    assert!(String::from_utf8_lossy(&env.ok(&[]).into_bytes()).contains("already running"));

    // After a crash the stale socket is replaced; bare `pawprint` detaches.
    first.0.kill().unwrap();
    first.0.wait().unwrap();
    assert_eq!(
        env.pawprint(&["status"]).status.code(),
        Some(4),
        "not running"
    );
    let out = env.ok(&[]);
    assert!(out.contains("PAWPRINT started"), "{out}");
    x.mapped_badge();
    env.ok(&["quit"]);
    wait_for("the detached badge to exit", || {
        x.badge().is_none().then_some(())
    });
    assert!(
        !env.runtime()
            .join("pawprint")
            .read_dir()
            .unwrap()
            .any(|_| true),
        "socket removed on quit"
    );
    assert!(first.exited().is_some());
}
