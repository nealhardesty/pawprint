//! X11 backend (SPEC §11.3), also used for GNOME on Wayland through
//! XWayland with the `gnome` profile (§11.5.1).
//!
//! A DOCK-type window kept above and on every desktop, with an empty XFixes
//! input shape for click-through. With a compositing manager it uses a
//! 32-bit ARGB visual; without one, the default visual plus a bounding shape
//! from the frame's 1-bit mask, which gives the same pixels because all art
//! is binary-alpha. With no window manager it is override-redirect.
//!
//! `pawprint preview` uses the same window with the `preview` flag: a
//! normal, managed, focusable window with a title, closed by the window
//! manager or Esc.

use std::collections::HashMap;
use std::os::fd::AsFd;

use calloop::generic::Generic;
use calloop::{EventLoop, Interest, Mode, PostAction};
use pawprint_core::badge::{Command, Event};
use pawprint_core::geom::{PhysicalRect, Point};
use pawprint_core::movemode::{Key, KeyCode};
use pawprint_core::render::Frame;
use pawprint_core::surface::{DisplayInfo, Surface};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::ReplyOrIdError;
use x11rb::properties::{WmHints, WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::linux::{self, App};
use crate::doctor::Report;
use crate::shell::Shell;
use crate::{CmdResult, exit, ipc};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_STATE,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_DESKTOP,
        _NET_WM_NAME,
        _NET_WM_PID,
        _NET_WORKAREA,
        _NET_CURRENT_DESKTOP,
        _NET_SUPPORTING_WM_CHECK,
        _MOTIF_WM_HINTS,
        UTF8_STRING,
        RESOURCE_MANAGER,
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
    }
}

type XResult<T> = Result<T, ReplyOrIdError>;

/// The connection and what we learned about the server.
struct Server {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    /// `_NET_WM_CM_S<screen>`, owned by the compositing manager.
    cm_atom: Atom,
    argb_visual: Option<Visualid>,
    root_depth: u8,
    root_visual: Visualid,
    randr_monitors: bool,
}

impl Server {
    fn open() -> XResult<Server> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|e| {
            ReplyOrIdError::ConnectionError(x11rb::errors::ConnectionError::IoError(
                std::io::Error::other(e.to_string()),
            ))
        })?;
        let screen = &conn.setup().roots[screen_num];
        let (root, root_depth, root_visual) = (screen.root, screen.root_depth, screen.root_visual);
        let argb_visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| &d.visuals)
            .find(|v| v.class == VisualClass::TRUE_COLOR)
            .map(|v| v.visual_id);
        let atoms = Atoms::new(&conn)?.reply()?;
        let cm_atom = conn
            .intern_atom(false, format!("_NET_WM_CM_S{screen_num}").as_bytes())?
            .reply()?
            .atom;
        // Extensions must be version-negotiated before use.
        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.shape_query_version()?.reply()?;
        let randr = conn.randr_query_version(1, 5)?.reply()?;
        Ok(Server {
            conn,
            root,
            atoms,
            cm_atom,
            argb_visual,
            root_depth,
            root_visual,
            randr_monitors: (randr.major_version, randr.minor_version) >= (1, 5),
        })
    }

    fn compositing(&self) -> XResult<bool> {
        Ok(self.conn.get_selection_owner(self.cm_atom)?.reply()?.owner != x11rb::NONE)
    }

    fn cardinals(&self, window: Window, prop: Atom, kind: impl Into<Atom>) -> XResult<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, window, prop, kind, 0, 1024)?
            .reply()?;
        Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
    }

    /// The EWMH window manager's check window, if one is running.
    fn wm_check(&self) -> XResult<Option<Window>> {
        let check = self.atoms._NET_SUPPORTING_WM_CHECK;
        let Some(&w) = self.cardinals(self.root, check, AtomEnum::WINDOW)?.first() else {
            return Ok(None);
        };
        // A stale property from a dead WM points at a window that is gone.
        let alive = self
            .cardinals(w, check, AtomEnum::WINDOW)
            .is_ok_and(|v| v.first() == Some(&w));
        Ok(alive.then_some(w))
    }

    fn wm_name(&self) -> XResult<Option<String>> {
        let Some(w) = self.wm_check()? else {
            return Ok(None);
        };
        let a = &self.atoms;
        let name = self
            .conn
            .get_property(false, w, a._NET_WM_NAME, a.UTF8_STRING, 0, 256)?
            .reply()?;
        Ok(Some(String::from_utf8_lossy(&name.value).into_owned()))
    }

    fn scale(&self) -> XResult<f64> {
        let a = self.atoms.RESOURCE_MANAGER;
        let res = self
            .conn
            .get_property(false, self.root, a, AtomEnum::STRING, 0, 1 << 16)?
            .reply()?;
        Ok(linux::dpi_scale(linux::xft_dpi(&String::from_utf8_lossy(
            &res.value,
        ))))
    }

    /// RandR monitors with `_NET_WORKAREA` as each one's usable area
    /// (SPEC §11.3.3).
    fn displays(&self) -> XResult<Vec<DisplayInfo>> {
        let scale = self.scale()?;
        let a = &self.atoms;
        let desktop = self
            .cardinals(self.root, a._NET_CURRENT_DESKTOP, AtomEnum::CARDINAL)?
            .first()
            .copied()
            .unwrap_or(0) as usize;
        let areas = self.cardinals(self.root, a._NET_WORKAREA, AtomEnum::CARDINAL)?;
        let workarea = areas
            .get(desktop * 4..desktop * 4 + 4)
            .or(areas.get(0..4))
            .map(|v| PhysicalRect::new(v[0] as i32, v[1] as i32, v[2], v[3]));
        let mut out = Vec::new();
        let monitors = if self.randr_monitors {
            self.conn
                .randr_get_monitors(self.root, true)?
                .reply()?
                .monitors
        } else {
            Vec::new()
        };
        for m in monitors {
            let name = self.conn.get_atom_name(m.name)?.reply()?.name;
            let bounds = PhysicalRect::new(m.x.into(), m.y.into(), m.width.into(), m.height.into());
            out.push(DisplayInfo {
                name: String::from_utf8_lossy(&name).into_owned(),
                bounds,
                work_area: workarea
                    .and_then(|w| intersect(w, bounds))
                    .unwrap_or(bounds),
                scale,
                primary: m.primary,
            });
        }
        if out.is_empty() {
            let s = &self.conn.setup().roots[0];
            let bounds =
                PhysicalRect::new(0, 0, s.width_in_pixels.into(), s.height_in_pixels.into());
            out.push(DisplayInfo {
                name: "screen".into(),
                bounds,
                work_area: workarea
                    .and_then(|w| intersect(w, bounds))
                    .unwrap_or(bounds),
                scale,
                primary: true,
            });
        }
        // No primary flagged: the monitor containing (0, 0).
        if !out.iter().any(|d| d.primary)
            && let Some(d) = out.iter_mut().find(|d| d.bounds.contains(Point::new(0, 0)))
        {
            d.primary = true;
        }
        Ok(out)
    }

    /// Arrow keys, Enter, and Escape by keycode, for move mode.
    fn key_map(&self) -> XResult<HashMap<Keycode, KeyCode>> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)?
            .reply()?;
        let per = usize::from(map.keysyms_per_keycode.max(1));
        let mut keys = HashMap::new();
        for (i, syms) in map.keysyms.chunks(per).enumerate() {
            let code = match syms[0] {
                0xff51 => KeyCode::Left,
                0xff52 => KeyCode::Up,
                0xff53 => KeyCode::Right,
                0xff54 => KeyCode::Down,
                0xff0d | 0xff8d => KeyCode::Enter,
                0xff1b => KeyCode::Escape,
                _ => continue,
            };
            keys.insert(min + i as u8, code);
        }
        Ok(keys)
    }
}

fn intersect(a: PhysicalRect, b: PhysicalRect) -> Option<PhysicalRect> {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (r, btm) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    (r > x && btm > y).then(|| PhysicalRect::new(x, y, (r - x) as u32, (btm - y) as u32))
}

/// The badge window: the X11 [`Surface`].
pub struct X11 {
    server: Server,
    gnome: bool,
    preview: bool,
    window: Window,
    /// The window manager's frame around ours, or the root.
    parent: Window,
    gc: Gcontext,
    argb: bool,
    override_redirect: bool,
    rect: Option<PhysicalRect>,
    frame: Option<Frame>,
    mapped: bool,
    interactive: bool,
    dragging: bool,
    keys: HashMap<Keycode, KeyCode>,
    displays: Vec<DisplayInfo>,
    /// The first error from a `Surface` call, reported by the next flush.
    error: Option<String>,
}

impl X11 {
    fn new(gnome: bool, preview: bool) -> XResult<X11> {
        let server = Server::open()?;
        let (conn, root) = (&server.conn, server.root);
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        conn.randr_select_input(
            root,
            randr::NotifyMask::SCREEN_CHANGE
                | randr::NotifyMask::CRTC_CHANGE
                | randr::NotifyMask::OUTPUT_CHANGE,
        )?;
        conn.xfixes_select_selection_input(
            root,
            server.cm_atom,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER
                | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?;
        let mut x = X11 {
            keys: server.key_map()?,
            displays: server.displays()?,
            server,
            gnome,
            preview,
            window: x11rb::NONE,
            parent: x11rb::NONE,
            gc: x11rb::NONE,
            argb: false,
            override_redirect: false,
            rect: None,
            frame: None,
            mapped: false,
            interactive: false,
            dragging: false,
            error: None,
        };
        x.create_window()?;
        Ok(x)
    }

    /// (Re)create the window for the current compositor and WM situation,
    /// restoring the last rect and frame.
    fn create_window(&mut self) -> XResult<()> {
        let s = &self.server;
        let conn = &s.conn;
        if self.window != x11rb::NONE {
            conn.destroy_window(self.window)?;
            conn.free_gc(self.gc)?;
            self.mapped = false;
        }
        // Mutter always composites (SPEC §11.5.1), so GNOME never needs the
        // shape fallback.
        // A preview's frames are opaque (drawn on a backdrop).
        let visual = s
            .argb_visual
            .filter(|_| !self.preview && (self.gnome || s.compositing().unwrap_or(false)));
        self.argb = visual.is_some();
        self.override_redirect = !self.preview && s.wm_check()?.is_none();
        log::info!(
            "x11: {} visual, {}",
            if self.argb {
                "32-bit ARGB"
            } else {
                "default (shaped)"
            },
            if self.override_redirect {
                "no window manager: override-redirect"
            } else {
                "EWMH window manager"
            }
        );
        let window = conn.generate_id()?;
        let (depth, visual_id) = match visual {
            Some(v) => (32, v),
            None => (s.root_depth, s.root_visual),
        };
        let mut aux = CreateWindowAux::new()
            .override_redirect(u32::from(self.override_redirect))
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::BUTTON_MOTION
                    | EventMask::KEY_PRESS
                    | EventMask::STRUCTURE_NOTIFY,
            );
        if let Some(v) = visual {
            let colormap = conn.generate_id()?;
            conn.create_colormap(ColormapAlloc::NONE, colormap, s.root, v)?;
            aux = aux.colormap(colormap).border_pixel(0).background_pixel(0);
        }
        let r = self.rect.unwrap_or(PhysicalRect::new(0, 0, 1, 1));
        conn.create_window(
            depth,
            window,
            s.root,
            r.x as i16,
            r.y as i16,
            r.width as u16,
            r.height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual_id,
            &aux,
        )?;
        let a = &s.atoms;
        let replace = PropMode::REPLACE;
        conn.change_property8(
            replace,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"pawprint\0Pawprint\0",
        )?;
        let title: &[u8] = if self.preview {
            b"PAWPRINT preview"
        } else {
            b"PAWPRINT"
        };
        conn.change_property8(replace, window, AtomEnum::WM_NAME, AtomEnum::STRING, title)?;
        conn.change_property8(replace, window, a._NET_WM_NAME, a.UTF8_STRING, title)?;
        conn.change_property32(
            replace,
            window,
            a._NET_WM_PID,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        self.window = window;
        self.parent = s.root;
        self.gc = conn.generate_id()?;
        conn.create_gc(self.gc, window, &CreateGCAux::new().graphics_exposures(0))?;
        if self.preview {
            // A normal window: closable through the window manager.
            conn.change_property32(
                replace,
                window,
                a.WM_PROTOCOLS,
                AtomEnum::ATOM,
                &[a.WM_DELETE_WINDOW],
            )?;
            if self.frame.is_some() {
                self.draw()?;
            }
            return Ok(());
        }
        conn.change_property32(
            replace,
            window,
            a._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[a._NET_WM_WINDOW_TYPE_DOCK],
        )?;
        let state = [
            a._NET_WM_STATE_ABOVE,
            a._NET_WM_STATE_STICKY,
            a._NET_WM_STATE_SKIP_TASKBAR,
            a._NET_WM_STATE_SKIP_PAGER,
        ];
        conn.change_property32(replace, window, a._NET_WM_STATE, AtomEnum::ATOM, &state)?;
        conn.change_property32(
            replace,
            window,
            a._NET_WM_DESKTOP,
            AtomEnum::CARDINAL,
            &[0xFFFF_FFFF],
        )?;
        // Motif hints: flags = decorations, decorations = none.
        conn.change_property32(
            replace,
            window,
            a._MOTIF_WM_HINTS,
            a._MOTIF_WM_HINTS,
            &[2, 0, 0, 0, 0],
        )?;
        WmHints {
            input: Some(false),
            ..WmHints::new()
        }
        .set(conn, window)?;
        self.set_input_shape()?;
        if self.frame.is_some() {
            self.draw()?;
        }
        Ok(())
    }

    /// Click-through: an empty input shape. Move mode: the whole badge.
    /// A preview keeps the default (the whole window).
    fn set_input_shape(&self) -> XResult<()> {
        if self.preview {
            return Ok(());
        }
        let conn = &self.server.conn;
        let region = conn.generate_id()?;
        let r = self.rect.unwrap_or_default();
        let rects: &[Rectangle] = if self.interactive {
            &[Rectangle {
                x: 0,
                y: 0,
                width: r.width as u16,
                height: r.height as u16,
            }]
        } else {
            &[]
        };
        conn.xfixes_create_region(region, rects)?;
        conn.xfixes_set_window_shape_region(self.window, shape::SK::INPUT, 0, 0, region)?;
        // Reparenting WMs (Openbox) frame even DOCK windows and don't copy
        // the input shape to the frame, which would then catch the clicks.
        if self.parent != self.server.root {
            conn.xfixes_set_window_shape_region(self.parent, shape::SK::INPUT, 0, 0, region)?;
        }
        conn.xfixes_destroy_region(region)?;
        Ok(())
    }

    /// Put the frame into the window (and its mask into the bounding shape
    /// without a compositor), then map it the first time.
    fn draw(&mut self) -> XResult<()> {
        let (Some(frame), Some(rect)) = (&self.frame, self.rect) else {
            return Ok(());
        };
        let s = &self.server;
        let conn = &s.conn;
        let stride = frame.width as usize * 4;
        let swap = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;
        let rows = ((conn.maximum_request_bytes() - 64) / stride).max(1);
        let depth = if self.argb { 32 } else { s.root_depth };
        for (i, chunk) in frame.bgra.chunks(rows * stride).enumerate() {
            let data: Vec<u8> = if swap {
                chunk
                    .chunks(4)
                    .flat_map(|p| [p[3], p[2], p[1], p[0]])
                    .collect()
            } else {
                chunk.to_vec()
            };
            let y = (i * rows) as i16;
            let h = (chunk.len() / stride) as u16;
            conn.put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                frame.width as u16,
                h,
                0,
                y,
                0,
                depth,
                &data,
            )?;
        }
        if !self.argb {
            let mut rects = Vec::new();
            for y in 0..frame.height {
                let mut x = 0;
                while x < frame.width {
                    if !frame.opaque(x, y) {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < frame.width && frame.opaque(x, y) {
                        x += 1;
                    }
                    rects.push(Rectangle {
                        x: start as i16,
                        y: y as i16,
                        width: (x - start) as u16,
                        height: 1,
                    });
                }
            }
            conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::BOUNDING,
                ClipOrdering::Y_SORTED,
                self.window,
                0,
                0,
                &rects,
            )?;
        }
        if !self.mapped {
            let spec = WmSizeHintsSpecification::UserSpecified;
            let fixed = self
                .preview
                .then_some((rect.width as i32, rect.height as i32));
            WmSizeHints {
                position: Some((spec, rect.x, rect.y)),
                size: Some((spec, rect.width as i32, rect.height as i32)),
                min_size: fixed,
                max_size: fixed,
                ..WmSizeHints::new()
            }
            .set_normal_hints(conn, self.window)?;
            conn.map_window(self.window)?;
            self.mapped = true;
            self.raise()?;
        }
        Ok(())
    }

    /// Without a window manager nothing keeps an override-redirect window
    /// on top, so raise it after mapping and after display changes.
    fn raise(&self) -> XResult<()> {
        if self.override_redirect && self.mapped {
            self.server.conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
        }
        Ok(())
    }

    fn record(&mut self, result: XResult<()>) {
        if let Err(e) = result {
            log::error!("x11: {e}");
            self.error.get_or_insert_with(|| e.to_string());
        }
    }

    /// Translate one X event; internal ones (expose, compositor changes)
    /// are handled here and give `None`.
    fn translate(&mut self, event: XEvent) -> XResult<Option<Event>> {
        let at = |x: i16, y: i16| Point::new(x.into(), y.into());
        Ok(match event {
            XEvent::Expose(e) if e.count == 0 => {
                self.draw()?;
                None
            }
            XEvent::ButtonPress(e) if self.interactive && e.detail == 1 => {
                let mask = EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION;
                self.server.conn.grab_pointer(
                    false,
                    self.window,
                    mask,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    x11rb::NONE,
                    e.time,
                )?;
                self.dragging = true;
                Some(Event::DragStart(at(e.root_x, e.root_y)))
            }
            XEvent::MotionNotify(e) if self.dragging => {
                Some(Event::DragMove(at(e.root_x, e.root_y)))
            }
            XEvent::ButtonRelease(e) if self.dragging && e.detail == 1 => {
                self.dragging = false;
                self.server.conn.ungrab_pointer(e.time)?;
                Some(Event::DragEnd(at(e.root_x, e.root_y)))
            }
            XEvent::KeyPress(e) if self.preview => (self.keys.get(&e.detail)
                == Some(&KeyCode::Escape))
            .then_some(Event::Command(Command::Quit)),
            XEvent::ClientMessage(e)
                if self.preview
                    && e.type_ == self.server.atoms.WM_PROTOCOLS
                    && e.data.as_data32()[0] == self.server.atoms.WM_DELETE_WINDOW =>
            {
                Some(Event::Command(Command::Quit))
            }
            XEvent::KeyPress(e) if self.interactive => self.keys.get(&e.detail).map(|&code| {
                Event::Key(Key {
                    code,
                    shift: u16::from(e.state) & u16::from(KeyButMask::SHIFT) != 0,
                })
            }),
            XEvent::ReparentNotify(e) if e.window == self.window => {
                self.parent = e.parent;
                self.set_input_shape()?;
                None
            }
            XEvent::MappingNotify(_) => {
                self.keys = self.server.key_map()?;
                None
            }
            XEvent::XfixesSelectionNotify(_) => {
                let argb = self.server.argb_visual.is_some()
                    && (self.gnome || self.server.compositing()?);
                if argb != self.argb {
                    log::info!(
                        "x11: compositing manager {}",
                        if argb { "started" } else { "stopped" }
                    );
                    self.create_window()?;
                }
                None
            }
            XEvent::PropertyNotify(e) => {
                let a = &self.server.atoms;
                let watched = [a._NET_WORKAREA, a._NET_CURRENT_DESKTOP, a.RESOURCE_MANAGER];
                watched
                    .contains(&e.atom)
                    .then(|| self.displays_changed())
                    .transpose()?
            }
            XEvent::RandrScreenChangeNotify(_) | XEvent::RandrNotify(_) => {
                Some(self.displays_changed()?)
            }
            XEvent::Error(e) => {
                log::warn!("x11: {e:?}");
                None
            }
            _ => None,
        })
    }

    fn displays_changed(&mut self) -> XResult<Event> {
        self.displays = self.server.displays()?;
        self.raise()?;
        Ok(Event::DisplaysChanged(self.displays.clone()))
    }
}

impl Surface for X11 {
    fn displays(&self) -> Vec<DisplayInfo> {
        self.displays.clone()
    }

    fn present(&mut self, frame: &pawprint_core::render::Frame) {
        self.frame = Some(frame.clone());
        let r = self.draw();
        self.record(r);
    }

    fn place(&mut self, rect: PhysicalRect) {
        let resized = self
            .rect
            .is_none_or(|r| (r.width, r.height) != (rect.width, rect.height));
        self.rect = Some(rect);
        let aux = ConfigureWindowAux::new()
            .x(rect.x)
            .y(rect.y)
            .width(rect.width)
            .height(rect.height);
        let mut r = self
            .server
            .conn
            .configure_window(self.window, &aux)
            .map(|_| ())
            .map_err(Into::into);
        if r.is_ok() && resized {
            r = self.set_input_shape();
        }
        self.record(r);
    }

    fn set_interactive(&mut self, on: bool) {
        self.interactive = on;
        self.dragging = false;
        let r = (|| -> XResult<()> {
            let conn = &self.server.conn;
            if on {
                // Arrow keys in move mode (SPEC §11.3.3). Fails harmlessly
                // if another client holds the keyboard.
                let grab = conn
                    .grab_keyboard(
                        false,
                        self.window,
                        x11rb::CURRENT_TIME,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )?
                    .reply()?;
                if grab.status != GrabStatus::SUCCESS {
                    log::info!(
                        "x11: keyboard grab refused ({:?}); drag still works",
                        grab.status
                    );
                }
            } else {
                conn.ungrab_keyboard(x11rb::CURRENT_TIME)?;
                conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
            }
            self.set_input_shape()
        })();
        self.record(r);
    }
}

struct X11App {
    shell: Shell,
    x: X11,
}

impl App for X11App {
    type Surface = X11;

    fn parts(&mut self) -> (&mut Shell, &mut X11) {
        (&mut self.shell, &mut self.x)
    }

    fn flush(&mut self) -> Result<(), String> {
        loop {
            let conn = &self.x.server.conn;
            conn.flush()
                .map_err(|e| format!("X11 connection lost: {e}"))?;
            let Some(event) = conn
                .poll_for_event()
                .map_err(|e| format!("X11 connection lost: {e}"))?
            else {
                break;
            };
            match self.x.translate(event) {
                Ok(Some(e)) => self.shell.handle(&mut self.x, e),
                Ok(None) => {}
                Err(e) => self.x.record(Err(e)),
            }
        }
        self.x.error.take().map_or(Ok(()), Err)
    }
}

pub fn run(shell: Shell, listener: Option<ipc::Listener>, gnome: bool) -> CmdResult {
    let no_x = |e: String| (exit::NO_SESSION, format!("cannot use the X11 display: {e}"));
    let x = X11::new(gnome, shell.is_preview()).map_err(|e| no_x(e.to_string()))?;
    let fd = x
        .server
        .conn
        .stream()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| no_x(e.to_string()))?;
    let event_loop = EventLoop::try_new().map_err(|e| (exit::ERROR, e.to_string()))?;
    // Wakes the loop; `X11App::flush` reads and handles the events.
    event_loop
        .handle()
        .insert_source(Generic::new(fd, Interest::READ, Mode::Level), |_, _, _| {
            Ok(PostAction::Continue)
        })
        .map_err(|e| (exit::ERROR, e.to_string()))?;
    let mut app = X11App { shell, x };
    app.shell.start(&mut app.x);
    linux::drive(event_loop, &mut app, listener)
}

/// Doctor rows for X11, plus its displays.
pub fn doctor(report: &mut Report) -> Vec<DisplayInfo> {
    let s = match Server::open() {
        Ok(s) => s,
        Err(e) => {
            report.problem(format!("cannot connect to the X11 display: {e}"));
            return Vec::new();
        }
    };
    let wm = s.wm_name().ok().flatten();
    report.row(
        "window manager",
        wm.as_deref()
            .map_or("none (override-redirect window)", |n| {
                if n.is_empty() { "EWMH (unnamed)" } else { n }
            }),
    );
    let compositing = s.compositing().unwrap_or(false);
    report.row(
        "compositor",
        if compositing {
            "running (_NET_WM_CM_S owned)"
        } else {
            "none"
        },
    );
    report.row(
        "ARGB visual",
        if s.argb_visual.is_some() { "yes" } else { "no" },
    );
    report.row(
        "transparency",
        if compositing && s.argb_visual.is_some() {
            "32-bit ARGB window"
        } else {
            "bounding shape from the badge mask (pixel-identical)"
        },
    );
    report.row(
        "RandR monitors",
        if s.randr_monitors {
            "yes (1.5)"
        } else {
            "no (one screen)"
        },
    );
    match s.displays() {
        Ok(d) => d,
        Err(e) => {
            report.problem(format!("cannot read X11 displays: {e}"));
            Vec::new()
        }
    }
}
