//! The X11 backend against a real X server (SPEC §11.3): Xvfb alone, with a
//! compositing manager, and with a window manager; plus what any Linux
//! backend shares: the StatusNotifierItem tray on a private session bus,
//! config edits and reloads, and `pawprint preview`. Needs `Xvfb`,
//! `xcompmgr`, `openbox`, and `dbus-daemon`; run with `make test-x11`.

#![cfg(target_os = "linux")]

mod common;

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::sync::{Arc, Mutex};
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
    /// Start Xvfb on a display it picks itself (`-displayfd`). Only clients
    /// with this test's cookie may connect. It listens on the filesystem
    /// socket, or on TCP where `/tmp/.X11-unix` is read-only (WSL), since
    /// x11rb can't use abstract sockets.
    fn start(env: &mut Env) -> X {
        let mut cookie = [0u8; 16];
        std::io::Read::read_exact(
            &mut std::fs::File::open("/dev/urandom").unwrap(),
            &mut cookie,
        )
        .unwrap();
        let auth = env.dir.join("Xauthority");
        std::fs::write(&auth, xauthority(&cookie)).unwrap();
        env.set("XAUTHORITY", auth.to_str().unwrap());

        let unix = writable(std::path::Path::new("/tmp/.X11-unix"));
        let (number, write) = std::io::pipe().unwrap();
        let fd = write.as_raw_fd();
        let mut cmd = env.command("Xvfb");
        cmd.args([
            "-displayfd",
            &fd.to_string(),
            "-auth",
            auth.to_str().unwrap(),
        ])
        .args(["-screen", "0", &format!("{W}x{H}x24")]);
        if unix {
            cmd.args(["-nolisten", "tcp"]);
        } else {
            cmd.args(["-listen", "tcp", "-nolisten", "unix"]);
        }
        // SAFETY: fcntl is async-signal-safe; it keeps the pipe open across exec.
        unsafe {
            cmd.pre_exec(move || match libc::fcntl(fd, libc::F_SETFD, 0) {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        let server = env.spawn_command(cmd);
        drop(write);
        // One line with the number, or EOF if Xvfb failed (see Xvfb.log).
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(number), &mut line).unwrap();
        let n: u32 = line.trim().parse().expect("Xvfb's display number");
        let display = if unix {
            format!(":{n}")
        } else {
            format!("127.0.0.1:{n}")
        };
        env.set("DISPLAY", &display);
        let (conn, screen) = connect(&display, &cookie).expect("a connection to Xvfb");
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

/// An Xauthority file with one MIT-MAGIC-COOKIE-1 entry for any address
/// and display (family Wild, empty number), so it works before the
/// display number is known. Every field is a big-endian length and bytes.
fn xauthority(cookie: &[u8]) -> Vec<u8> {
    let mut out = 0xffffu16.to_be_bytes().to_vec();
    for field in [&b""[..], b"", b"MIT-MAGIC-COOKIE-1", cookie] {
        out.extend((field.len() as u16).to_be_bytes());
        out.extend(field);
    }
    out
}

fn writable(dir: &std::path::Path) -> bool {
    let path = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: access only reads the NUL-terminated path.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Connect to `display` with `cookie`; the test's own connection doesn't
/// read XAUTHORITY, which is set for the children only.
fn connect(display: &str, cookie: &[u8]) -> Option<(RustConnection, usize)> {
    use x11rb::reexports::x11rb_protocol::parse_display::parse_display;
    let parsed = parse_display(Some(display)).ok()?;
    let screen = usize::from(parsed.screen);
    parsed.connect_instruction().find_map(|addr| {
        let (stream, _) = x11rb::rust_connection::DefaultStream::connect(&addr).ok()?;
        RustConnection::connect_to_stream_with_auth_info(
            stream,
            screen,
            b"MIT-MAGIC-COOKIE-1".to_vec(),
            cookie.to_vec(),
        )
        .ok()
        .map(|c| (c, screen))
    })
}

/// What a desktop panel provides: a StatusNotifierWatcher. It records the
/// bus names of the items that register.
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: &str) {
        self.items.lock().unwrap().push(service.to_string());
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }
}

const SNI: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";

/// A dbusmenu layout node: id, properties, children.
/// An SNI pixmap: width, height, ARGB32 bytes.
type Pixmap = (i32, i32, Vec<u8>);
/// An SNI tooltip: icon name, pixmaps, title, description.
type ToolTip = (String, Vec<Pixmap>, String, String);

type Node = (
    i32,
    HashMap<String, zbus::zvariant::OwnedValue>,
    Vec<zbus::zvariant::OwnedValue>,
);

/// A private session bus (`dbus-daemon`) with a [`Watcher`] on it.
struct Bus {
    conn: zbus::blocking::Connection,
    items: Arc<Mutex<Vec<String>>>,
    _daemon: Proc,
}

impl Bus {
    fn start(env: &mut Env) -> Bus {
        let addr = format!("unix:path={}", env.dir.join("bus").display());
        let daemon = env.spawn(
            "dbus-daemon",
            &[
                "--session",
                "--nofork",
                "--nopidfile",
                &format!("--address={addr}"),
            ],
        );
        env.set("DBUS_SESSION_BUS_ADDRESS", &addr);
        let items = Arc::new(Mutex::new(Vec::new()));
        let conn = wait_for("dbus-daemon", || {
            zbus::blocking::connection::Builder::address(addr.as_str())
                .ok()?
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        items: items.clone(),
                    },
                )
                .ok()?
                .name("org.kde.StatusNotifierWatcher")
                .ok()?
                .build()
                .ok()
        });
        Bus {
            conn,
            items,
            _daemon: daemon,
        }
    }

    /// The bus name of the newest registered item other than `not`.
    fn item(&self, not: Option<&str>) -> String {
        wait_for("the badge's StatusNotifierItem", || {
            self.items
                .lock()
                .unwrap()
                .last()
                .filter(|i| Some(i.as_str()) != not && self.has_owner(i))
                .cloned()
        })
    }

    fn has_owner(&self, name: &str) -> bool {
        let dbus = zbus::blocking::fdo::DBusProxy::new(&self.conn).unwrap();
        dbus.name_has_owner(name.try_into().unwrap())
            .unwrap_or(false)
    }

    fn property<T>(&self, item: &str, name: &str) -> T
    where
        T: TryFrom<zbus::zvariant::OwnedValue>,
        T::Error: std::fmt::Debug,
    {
        let props = zbus::blocking::fdo::PropertiesProxy::builder(&self.conn)
            .destination(item.to_string())
            .unwrap()
            .path("/StatusNotifierItem")
            .unwrap()
            .build()
            .unwrap();
        let value = props.get(SNI.try_into().unwrap(), name).unwrap();
        T::try_from(value).unwrap()
    }

    fn layout(&self, item: &str) -> Node {
        let reply = self
            .conn
            .call_method(
                Some(item),
                "/MenuBar",
                Some(MENU),
                "GetLayout",
                &(0i32, -1i32, Vec::<String>::new()),
            )
            .unwrap();
        reply.body().deserialize::<(u32, Node)>().unwrap().1
    }

    /// The menu item at `path` (labels): its id and whether it is checked.
    fn find(&self, item: &str, path: &[&str]) -> (i32, bool) {
        fn walk(node: &Node, path: &[&str]) -> Option<(i32, bool)> {
            let [first, rest @ ..] = path else {
                let checked = node
                    .1
                    .get("toggle-state")
                    .and_then(|v| i32::try_from(v).ok());
                return Some((node.0, checked == Some(1)));
            };
            node.2.iter().find_map(|child| {
                let child = Node::try_from(child.try_clone().ok()?).ok()?;
                let label = child
                    .1
                    .get("label")
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())?;
                (label == *first).then(|| walk(&child, rest)).flatten()
            })
        }
        walk(&self.layout(item), path).unwrap_or_else(|| panic!("no menu item {path:?}"))
    }

    fn click(&self, item: &str, path: &[&str]) {
        let (id, _) = self.find(item, path);
        let data = zbus::zvariant::Value::from(0i32);
        self.conn
            .call_method(
                Some(item),
                "/MenuBar",
                Some(MENU),
                "Event",
                &(id, "clicked", data, 0u32),
            )
            .unwrap();
    }

    /// Click an item that ends the badge (Quit), asking for no reply: the
    /// badge may exit before ksni answers, which a call reports as NoReply.
    fn click_last(&self, item: &str, path: &[&str]) {
        let (id, _) = self.find(item, path);
        let data = zbus::zvariant::Value::from(0i32);
        let msg = zbus::Message::method_call("/MenuBar", "Event")
            .unwrap()
            .destination(item)
            .unwrap()
            .interface(MENU)
            .unwrap()
            .with_flags(zbus::message::Flags::NoReplyExpected)
            .unwrap()
            .build(&(id, "clicked", data, 0u32))
            .unwrap();
        self.conn.send(&msg).unwrap();
    }
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
    // No session bus: no tray, and doctor says so.
    let doctor = env.ok(&["doctor"]);
    assert!(doctor.contains("no tray"), "{doctor}");
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
    // Move mode shows the dashed accent outline (SPEC §6.3): a dash in the
    // transparent top-left corner, the art through the gaps.
    let accent = wait_for("the move-mode outline", || {
        let p = x.capture().px(268, 358);
        (p[..3] != BACKGROUND).then_some(p)
    });
    let screen = x.capture();
    assert_eq!(screen.px(268 + 63, 358), accent, "top-right dash");
    if expected.px(4, 0)[3] == 0 {
        assert_eq!(screen.px(268 + 4, 358)[..3], BACKGROUND, "a gap");
    }
    x.key(0xff1b, false);
    env.wait_rect([267, 358, 64, 64]);
    wait_for("the outline to go", || {
        (x.capture().px(267, 358)[..3] == BACKGROUND).then_some(())
    });

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
    wait_for("shaped pixels", || {
        let screen = x.capture();
        (screen.px(1232, 60)[..3] == expected.px(32, 44)[..3]).then_some(())
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
fn a_refused_request_does_not_end_the_badge() {
    let mut env = Env::new("x11-refused");
    let x = X::start(&mut env);
    let mut badge = env.run_badge();
    let win = x.mapped_badge();
    env.wait_rect([1200, 16, 64, 64]);
    // Another client destroys the window, so move mode's keyboard grab
    // (a request with a reply) gets BadWindow.
    x.conn.destroy_window(win).unwrap();
    x.conn.sync().unwrap();
    env.ok(&["move"]);
    env.ok(&["status"]);
    assert!(badge.exited().is_none(), "{}", env.log());
    assert!(env.log().contains("request failed"), "{}", env.log());
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

    // A badge that doesn't answer (stopped, or stuck) still owns the
    // session: a second one must not replace it (SPEC §10.4).
    let pid = first.0.id() as i32;
    // SAFETY: kill only sends a signal.
    unsafe { libc::kill(pid, libc::SIGSTOP) };
    let mut second = Proc(
        env.command(env!("CARGO_BIN_EXE_pawprint"))
            .arg("run")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let code = wait_for("the second badge to give up", || second.exited());
    unsafe { libc::kill(pid, libc::SIGCONT) };
    let mut out = String::new();
    std::io::Read::read_to_string(&mut second.0.stdout.take().unwrap(), &mut out).unwrap();
    assert!(code.success() && out.contains("already running"), "{out}");
    let status = env.ok(&["status"]);
    assert!(status.contains(&format!("pid {pid}")), "{status}");

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
        !env.runtime().join("pawprint").read_dir().unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "sock")),
        "socket removed on quit"
    );
    assert!(first.exited().is_some());
}

#[test]
#[ignore = "needs Xvfb and dbus-daemon; run `make test-x11`"]
fn tray_menu_and_config_edits_reach_the_badge() {
    let mut env = Env::new("x11-tray");
    let x = X::start(&mut env);
    x.background(false, (0, 0, W, H), BG_PIXEL);
    let bus = Bus::start(&mut env);

    // `config init` writes every default once.
    env.ok(&["config", "init"]);
    let template = env.config();
    assert!(template.contains("size = 64") && template.contains("tray = true"));
    assert!(
        !env.pawprint(&["config", "init"]).status.success(),
        "never over a file"
    );

    let mut badge = env.run_badge();
    x.mapped_badge();
    env.wait_rect([1200, 16, 64, 64]);

    // The StatusNotifierItem: identity, tooltip, and the rest frame as icon.
    let item = bus.item(None);
    assert_eq!(bus.property::<String>(&item, "Id"), "pawprint");
    assert_eq!(bus.property::<String>(&item, "Title"), "PAWPRINT");
    let icons: Vec<Pixmap> = bus.property(&item, "IconPixmap");
    let sizes: Vec<(i32, i32)> = icons.iter().map(|i| (i.0, i.1)).collect();
    assert_eq!(sizes, [(32, 32), (64, 64)]);
    assert!(icons[0].2.chunks(4).any(|p| p[0] == 255), "an opaque pixel");
    let (_, _, title, _): ToolTip = bus.property(&item, "ToolTip");
    assert_eq!(title, "PAWPRINT: OWL (owl)");

    let doctor = env.ok(&["doctor"]);
    assert!(
        doctor.contains("StatusNotifierItem host available"),
        "{doctor}"
    );

    // Autostart: an XDG entry that runs this binary.
    assert!(env.ok(&["autostart", "status"]).starts_with("off"));
    env.ok(&["autostart", "on"]);
    let entry =
        std::fs::read_to_string(env.home().join(".config/autostart/pawprint.desktop")).unwrap();
    assert!(
        entry.contains(&format!("Exec={} run\n", env!("CARGO_BIN_EXE_pawprint"))),
        "{entry}"
    );
    assert!(env.ok(&["doctor"]).contains("run at login"));
    env.ok(&["autostart", "off"]);
    assert!(env.ok(&["autostart", "status"]).starts_with("off"));

    // The shared menu, checked where the settings are.
    assert!(bus.find(&item, &["Size", "64"]).1);
    assert!(bus.find(&item, &["Animation", "Idle"]).1);

    // Size > 128: the badge grows, and the choice lands in this host's table
    // without touching the rest of the file.
    bus.click(&item, &["Size", "128"]);
    env.wait_rect([1280 - 128 - 16, 16, 128, 128]);
    let config = env.config();
    assert!(config.starts_with(&template), "{config}");
    assert!(
        config.ends_with("[hosts]\n\n[hosts.owl]\nsize = 128\n"),
        "{config}"
    );
    wait_for("the 128 check", || {
        bus.find(&item, &["Size", "128"]).1.then_some(())
    });
    bus.click(&item, &["Animation", "Static"]);
    wait_for("animation saved", || {
        env.config()
            .contains("animation = \"static\"")
            .then_some(())
    });

    // `pawprint set` and `unset` reach the running badge too.
    let out = env.ok(&["set", "size", "32"]);
    assert!(out.contains("reloaded"), "{out}");
    env.wait_rect([1280 - 32 - 16, 16, 32, 32]);
    env.ok(&["unset", "size"]);
    env.wait_rect([1200, 16, 64, 64]);
    let bad = env.pawprint(&["set", "mascot", "no-such-animal"]);
    assert_eq!(bad.status.code(), Some(2), "unknown mascots are refused");
    assert!(!env.config().contains("no-such-animal"));
    // A user mascot added while the badge runs: the reload re-reads the
    // folder, so `set` shows it at once (SPEC §12.7).
    let zork = env.home().join(".config/pawprint/mascots/zork");
    env.ok(&["mascot", "export", "walrus", zork.to_str().unwrap()]);
    let toml = zork.join("mascot.toml");
    let text = std::fs::read_to_string(&toml).unwrap();
    std::fs::write(&toml, text.replace("id = \"walrus\"", "id = \"zork\"")).unwrap();
    env.ok(&["set", "mascot", "zork"]);
    assert!(env.ok(&["status"]).contains("mascot      zork"));
    env.ok(&["unset", "mascot"]);
    assert!(env.ok(&["forget"]).contains("Nothing cached for owl"));

    // A hand edit is picked up at the next event, here an IPC request
    // (mtime check, SPEC §8.5).
    env.write_config(
        &env.config()
            .replace("position = \"top-right\"", "position = \"top-left\""),
    );
    env.ok(&["status"]);
    env.wait_rect([16, 16, 64, 64]);

    // tray = false removes the item; turning it back on brings a new one.
    env.ok(&["set", "tray", "false"]);
    wait_for("the item to go", || (!bus.has_owner(&item)).then_some(()));
    env.ok(&["unset", "tray"]);
    let item = bus.item(Some(&item));

    // Quit from the menu; the saved choice outlives the badge.
    bus.click_last(&item, &["Quit"]);
    wait_for("the badge to exit", || badge.exited());
    let _badge = env.run_badge();
    env.wait_rect([16, 16, 64, 64]);
    let explain = env.ok(&["explain"]);
    assert!(
        explain
            .lines()
            .any(|l| l.starts_with("animation") && l.contains("static")),
        "{explain}"
    );
}

#[test]
#[ignore = "needs Xvfb; run `make test-x11`"]
fn preview_is_a_normal_window_that_cycles_and_closes() {
    const BACKDROP: [u8; 3] = [0x5a, 0x60, 0x6b];
    let mut env = Env::new("x11-preview");
    let x = X::start(&mut env);
    x.background(false, (0, 0, W, H), BG_PIXEL);
    assert!(
        !env.pawprint(&["preview", "no-such-animal"])
            .status
            .success()
    );

    let preview = |env: &Env| {
        env.spawn(
            env!("CARGO_BIN_EXE_pawprint"),
            &["preview", "owl", "--size", "64"],
        )
    };
    let mut first = preview(&env);
    let win = x.mapped_badge();
    let attrs = x.conn.get_window_attributes(win).unwrap().reply().unwrap();
    assert!(!attrs.override_redirect, "a normal window");
    assert!(x.atoms(win, "_NET_WM_WINDOW_TYPE").is_empty(), "not a dock");
    assert!(
        x.atoms(win, "WM_PROTOCOLS")
            .contains(&x.atom("WM_DELETE_WINDOW"))
    );

    // Centered, the badge on a backdrop with a 16 px margin.
    let geo = x.conn.get_geometry(win).unwrap().reply().unwrap();
    let (wx, wy): (i16, i16) = ((1280 - 96) / 2, (720 - 96) / 2);
    assert_eq!((geo.x, geo.y, geo.width, geo.height), (wx, wy, 96, 96));
    let expected = env.expected(64);
    let rest = wait_for("the preview on screen", || {
        let s = x.capture();
        (s.px(wx as u32, wy as u32)[..3] == BACKDROP).then_some(s)
    });
    let (bx, by) = (i32::from(wx) + 16, i32::from(wy) + 16);
    assert_badge_at(&rest, &expected, bx, by, BACKDROP);

    // Every action in turn: the frame changes within a few seconds.
    wait_for("an action to play", || {
        let s = x.capture();
        let changed = (0..64).any(|y| {
            (0..64).any(|dx| {
                let (sx, sy) = ((bx + dx) as u32, (by + y) as u32);
                s.px(sx, sy) != rest.px(sx, sy)
            })
        });
        changed.then_some(())
    });

    // Esc closes it (the pointer gives it the keyboard without a WM).
    x.pointer_to(wx + 48, wy + 48);
    x.key(0xff1b, false);
    let status = wait_for("Esc to close the preview", || first.exited());
    assert!(status.success());

    // So does the window manager's close button (WM_DELETE_WINDOW).
    let mut second = preview(&env);
    let win = x.mapped_badge();
    let event = ClientMessageEvent::new(
        32,
        win,
        x.atom("WM_PROTOCOLS"),
        [x.atom("WM_DELETE_WINDOW"), 0, 0, 0, 0],
    );
    x.conn
        .send_event(false, win, EventMask::NO_EVENT, event)
        .unwrap();
    x.conn.flush().unwrap();
    let status = wait_for("WM_DELETE_WINDOW to close the preview", || second.exited());
    assert!(status.success());
}
