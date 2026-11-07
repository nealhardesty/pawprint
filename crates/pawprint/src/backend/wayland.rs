//! Wayland backend (SPEC §11.4): a `zwlr_layer_shell_v1` overlay surface for
//! Plasma and wlroots compositors.
//!
//! Coordinates: the core works in physical pixels, but Wayland never tells
//! a client where anything is. Each output's bounds are its logical
//! position and size times its scale. Its usable area comes from a *probe*:
//! an unmapped layer surface anchored to all four edges with exclusive zone
//! 0, which the compositor sizes to the area left over by panels. Only the
//! size is known, so the usable area is placed at the output's origin, and
//! the badge is positioned by margins measured from the anchored edges,
//! which the compositor applies relative to the real usable area (F3).

use std::collections::HashMap;

use calloop::EventLoop;
use pawprint_core::badge::Event;
use pawprint_core::geom::{PhysicalRect, Point};
use pawprint_core::layout::{self, Axis};
use pawprint_core::movemode::{Key, KeyCode};
use pawprint_core::render::Frame;
use pawprint_core::surface::{DisplayInfo, Surface};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputInfo, OutputState};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_plasma::output_order::v1::client::kde_output_order_v1;

use super::linux::{self, App};
use crate::doctor::Report;
use crate::shell::Shell;
use crate::{CmdResult, exit, ipc};

/// The layer-shell namespace (SPEC §2).
const NAMESPACE: &str = "pawprint";
/// Left mouse button (linux/input-event-codes.h).
const BTN_LEFT: u32 = 0x110;

/// The interface names the compositor advertises.
pub fn globals() -> Result<Vec<String>, String> {
    struct Probe;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
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
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, _) = registry_queue_init::<Probe>(&conn).map_err(|e| e.to_string())?;
    Ok(globals
        .contents()
        .with_list(|l| l.iter().map(|g| g.interface.clone()).collect()))
}

/// The usable-area probe for one output.
struct Probe {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    /// Logical size of the usable area, once configured.
    usable: Option<(u32, u32)>,
}

/// The badge's layer surface. Its output is fixed at creation (F5).
struct Badge {
    layer: LayerSurface,
    output: String,
    viewport: Option<wp_viewport::WpViewport>,
    configured: bool,
    buffer: Option<Buffer>,
}

/// Move mode's input surface: transparent, covering the usable area of the
/// badge's output, above everything. The badge itself never takes input.
///
/// Why not drag the badge surface: during the implicit grab, sway keeps
/// reporting pointer positions relative to where the surface was at the
/// press, not where it is now, so moving the surface under the pointer
/// makes the drag run away (F7). The catcher never moves, and its local
/// coordinates are relative to the usable area, exactly the core's frame.
struct Catcher {
    layer: LayerSurface,
    viewport: Option<wp_viewport::WpViewport>,
    display: DisplayInfo,
    buffer: Option<Buffer>,
}

/// Everything but the shell: the Wayland [`Surface`].
pub struct Wl {
    conn: Connection,
    qh: QueueHandle<WlApp>,
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    compositor: CompositorState,
    layer_shell: Option<LayerShell>,
    shm: Shm,
    pool: SlotPool,
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// Output names in Plasma's priority order; the first is the primary.
    order: Vec<String>,
    order_next: Vec<String>,
    probes: Vec<Probe>,
    badge: Option<Badge>,
    catcher: Option<Catcher>,
    rect: Option<PhysicalRect>,
    frame: Option<Frame>,
    new_frame: bool,
    dirty: bool,
    interactive: bool,
    dragging: bool,
    shift: bool,
    /// Move-mode keys by wl_keyboard keycode, from the current keymap.
    keys: HashMap<u32, KeyCode>,
    displays: Vec<DisplayInfo>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
}

/// `shell` is `None` when `doctor` only looks at the outputs.
pub struct WlApp {
    shell: Option<Shell>,
    wl: Wl,
}

/// One output as a display. Bounds are logical position and size times
/// the scale; the usable area is the probe's size at the same origin.
///
/// The scale is the physical mode over the xdg-output logical size, which
/// is exactly the compositor's fractional scale, so wp_fractional_scale_v1
/// adds nothing: the core picks the buffer size and the viewport maps it.
fn display_info(
    info: &OutputInfo,
    viewporter: bool,
    usable: Option<(u32, u32)>,
    primary: bool,
) -> DisplayInfo {
    use wl_output::Transform as T;
    let mode = info
        .modes
        .iter()
        .find(|m| m.current)
        .map_or((0, 0), |m| m.dimensions);
    let (mw, mh) = match info.transform {
        T::_90 | T::_270 | T::Flipped90 | T::Flipped270 => (mode.1, mode.0),
        _ => mode,
    };
    let int = info.scale_factor.max(1);
    let (lw, lh) = info.logical_size.unwrap_or((mw / int, mh / int));
    // With a viewport any scale renders crisply; without one, only the
    // integer buffer scale exists.
    let scale = if viewporter {
        linux::mode_scale(mw, lw)
    } else {
        f64::from(int)
    };
    let (lx, ly) = info.logical_position.unwrap_or(info.location);
    let px = |v: i32| (f64::from(v) * scale).round() as i32;
    let bounds = PhysicalRect::new(px(lx), px(ly), px(lw).max(1) as u32, px(lh).max(1) as u32);
    let work_area = usable
        .filter(|&(w, h)| w > 0 && h > 0)
        .map_or(bounds, |(w, h)| {
            PhysicalRect::new(bounds.x, bounds.y, px(w as i32) as u32, px(h as i32) as u32)
        });
    DisplayInfo {
        name: info
            .name
            .clone()
            .unwrap_or_else(|| format!("output-{}", info.id)),
        bounds,
        work_area,
        scale,
        primary,
    }
}

impl Wl {
    fn output_named(&self, name: &str) -> Option<wl_output::WlOutput> {
        self.outputs
            .outputs()
            .find(|o| self.outputs.info(o).and_then(|i| i.name).as_deref() == Some(name))
    }

    fn refresh_displays(&mut self) {
        let mut displays: Vec<DisplayInfo> = self
            .outputs
            .outputs()
            .filter_map(|o| {
                let info = self.outputs.info(&o)?;
                let usable = self
                    .probes
                    .iter()
                    .find(|p| p.output == o)
                    .and_then(|p| p.usable);
                let name = info.name.clone().unwrap_or_default();
                let primary = self.order.first() == Some(&name);
                Some(display_info(
                    &info,
                    self.viewporter.is_some(),
                    usable,
                    primary,
                ))
            })
            .collect();
        displays.sort_by_key(|d| (d.bounds.x, d.bounds.y));
        self.displays = displays;
    }

    fn empty_region(&self) -> Option<Region> {
        Region::new(&self.compositor).ok()
    }

    fn add_probe(&mut self, output: wl_output::WlOutput) {
        let Some(shell) = &self.layer_shell else {
            return;
        };
        let surface = self.compositor.create_surface(&self.qh);
        let layer = shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some("pawprint-workarea"),
            Some(&output),
        );
        layer.set_anchor(Anchor::all());
        layer.set_size(0, 0);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        if let Some(r) = self.empty_region() {
            layer.set_input_region(Some(r.wl_region()));
        }
        layer.commit();
        self.probes.push(Probe {
            output,
            layer,
            usable: None,
        });
    }

    /// The display the badge rect is on.
    fn display_for(&self, rect: PhysicalRect) -> Option<&DisplayInfo> {
        let c = rect.center();
        self.displays
            .iter()
            .find(|d| d.bounds.contains(c))
            .or(self.displays.first())
    }

    fn create_badge(&mut self, display: &DisplayInfo) {
        let (Some(shell), Some(output)) = (&self.layer_shell, self.output_named(&display.name))
        else {
            return;
        };
        let surface = self.compositor.create_surface(&self.qh);
        let viewport = self
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, &self.qh, ()));
        let layer = shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some(NAMESPACE),
            Some(&output),
        );
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // A first commit without a buffer asks for the initial configure.
        layer.commit();
        log::info!("wayland: badge surface on {}", display.name);
        self.badge = Some(Badge {
            layer,
            output: display.name.clone(),
            viewport,
            configured: false,
            buffer: None,
        });
    }

    /// Bring the compositor up to date with the last place, present, and
    /// interactive state, in a single commit.
    fn sync(&mut self) {
        let (Some(rect), true) = (self.rect, self.frame.is_some()) else {
            return;
        };
        let Some(display) = self.display_for(rect).cloned() else {
            return;
        };
        if self
            .badge
            .as_ref()
            .is_some_and(|b| b.output != display.name)
        {
            self.badge = None; // its output is fixed, so make a new one
        }
        if self.badge.is_none() {
            self.create_badge(&display);
            self.dirty = true;
            return;
        }
        let empty = self.empty_region();
        let Some(badge) = self.badge.as_mut().filter(|b| b.configured && self.dirty) else {
            return;
        };
        let (s, work) = (display.scale, display.work_area);
        let logical = |p: i32| (f64::from(p) / s).round() as i32;
        // Anchor to the edges nearest the badge (the core's own 3 x 3 rule)
        // and measure margins from them.
        let (h, v) = layout::snap(rect, work, s).anchor.axes();
        let (mut anchor, mut m) = (Anchor::empty(), [0; 4]); // top, right, bottom, left
        match h {
            Axis::End => {
                (anchor, m[1]) = (anchor | Anchor::RIGHT, logical(work.right() - rect.right()))
            }
            _ => (anchor, m[3]) = (anchor | Anchor::LEFT, logical(rect.x - work.x)),
        }
        match v {
            Axis::End => {
                (anchor, m[2]) = (
                    anchor | Anchor::BOTTOM,
                    logical(work.bottom() - rect.bottom()),
                )
            }
            _ => (anchor, m[0]) = (anchor | Anchor::TOP, logical(rect.y - work.y)),
        }
        let (w, hgt) = (
            logical(rect.width as i32).max(1),
            logical(rect.height as i32).max(1),
        );
        badge.layer.set_anchor(anchor);
        badge.layer.set_margin(m[0], m[1], m[2], m[3]);
        badge.layer.set_size(w as u32, hgt as u32);
        let surface = badge.layer.wl_surface().clone();
        let frame = self.frame.as_ref().expect("checked above");
        match &badge.viewport {
            Some(vp) => vp.set_destination(w, hgt),
            None => {
                let int = s as i32;
                let fits = frame.width as i32 % int == 0 && frame.height as i32 % int == 0;
                surface.set_buffer_scale(if fits { int } else { 1 });
            }
        }
        if self.new_frame {
            let (fw, fh) = (frame.width as i32, frame.height as i32);
            match self
                .pool
                .create_buffer(fw, fh, fw * 4, wl_shm::Format::Argb8888)
            {
                Ok((buffer, canvas)) => {
                    canvas.copy_from_slice(&frame.bgra);
                    if buffer.attach_to(&surface).is_ok() {
                        surface.damage_buffer(0, 0, fw, fh);
                        badge.buffer = Some(buffer);
                    }
                    self.new_frame = false;
                }
                Err(e) => log::error!("wayland: cannot allocate a buffer: {e}"),
            }
        }
        // Click-through: an empty input region (F4), always; move mode
        // takes input through the catcher.
        surface.set_input_region(empty.as_ref().map(Region::wl_region));
        badge.layer.commit();
        self.dirty = false;
        if !self.interactive {
            self.catcher = None;
        } else if self.catcher.is_none() {
            self.create_catcher(display);
        }
    }

    fn create_catcher(&mut self, display: DisplayInfo) {
        let (Some(shell), Some(output)) = (&self.layer_shell, self.output_named(&display.name))
        else {
            return;
        };
        let surface = self.compositor.create_surface(&self.qh);
        let viewport = self
            .viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, &self.qh, ()));
        let layer = shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some("pawprint-move"),
            Some(&output),
        );
        layer.set_anchor(Anchor::all());
        layer.set_size(0, 0);
        layer.set_exclusive_zone(0);
        // Exclusive, like the X11 keyboard grab: arrow keys work at once.
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        self.catcher = Some(Catcher {
            layer,
            viewport,
            display,
            buffer: None,
        });
    }

    /// Fill the configured catcher with transparent pixels: one pixel
    /// stretched by the viewport, or a full-size buffer without one.
    fn fill_catcher(&mut self, (w, h): (u32, u32)) {
        let Some(c) = self.catcher.as_mut() else {
            return;
        };
        let (bw, bh) = if c.viewport.is_some() {
            (1, 1)
        } else {
            (w.max(1) as i32, h.max(1) as i32)
        };
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(bw, bh, bw * 4, wl_shm::Format::Argb8888)
        else {
            return;
        };
        canvas.fill(0);
        let surface = c.layer.wl_surface();
        if let Some(vp) = &c.viewport {
            vp.set_destination(w.max(1) as i32, h.max(1) as i32);
        }
        if buffer.attach_to(surface).is_ok() {
            surface.damage_buffer(0, 0, bw, bh);
            c.buffer = Some(buffer);
        }
        c.layer.commit();
    }
}

impl Surface for Wl {
    fn displays(&self) -> Vec<DisplayInfo> {
        self.displays.clone()
    }

    fn present(&mut self, frame: &Frame) {
        self.frame = Some(frame.clone());
        self.new_frame = true;
        self.dirty = true;
    }

    fn place(&mut self, rect: PhysicalRect) {
        self.rect = Some(rect);
        self.dirty = true;
    }

    fn set_interactive(&mut self, on: bool) {
        self.interactive = on;
        self.dragging = false;
        self.dirty = true;
    }
}

impl WlApp {
    fn send(&mut self, event: Event) {
        if let Some(shell) = &mut self.shell {
            shell.handle(&mut self.wl, event);
        }
    }

    fn displays_changed(&mut self) {
        self.wl.refresh_displays();
        self.send(Event::DisplaysChanged(self.wl.displays.clone()));
    }

    /// Connect, bind, and wait until outputs and usable areas are known.
    fn connect(shell: Option<Shell>) -> Result<(WlApp, EventQueue<WlApp>), String> {
        let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
        let (globals, mut queue) =
            registry_queue_init::<WlApp>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let compositor =
            CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
        let pool = SlotPool::new(64 * 64 * 4, &shm).map_err(|e| format!("shm pool: {e}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh).ok();
        if shell.is_some() && layer_shell.is_none() {
            return Err("the compositor has no zwlr_layer_shell_v1".into());
        }
        let viewporter = globals.bind(&qh, 1..=1, ()).ok();
        let _order: Option<kde_output_order_v1::KdeOutputOrderV1> =
            globals.bind(&qh, 1..=1, ()).ok();
        let mut app = WlApp {
            shell,
            wl: Wl {
                conn: conn.clone(),
                qh: qh.clone(),
                registry: RegistryState::new(&globals),
                outputs: OutputState::new(&globals, &qh),
                seats: SeatState::new(&globals, &qh),
                compositor,
                layer_shell,
                shm,
                pool,
                viewporter,
                order: Vec::new(),
                order_next: Vec::new(),
                probes: Vec::new(),
                badge: None,
                catcher: None,
                rect: None,
                frame: None,
                new_frame: false,
                dirty: false,
                interactive: false,
                dragging: false,
                shift: false,
                keys: move_keys(None),
                displays: Vec::new(),
                pointer: None,
                keyboard: None,
            },
        };
        // Outputs, then xdg-output details, then the probes' configures.
        for _ in 0..3 {
            queue.roundtrip(&mut app).map_err(|e| e.to_string())?;
        }
        app.wl.refresh_displays();
        Ok((app, queue))
    }
}

impl App for WlApp {
    type Surface = Wl;

    fn parts(&mut self) -> (&mut Shell, &mut Wl) {
        (
            self.shell.as_mut().expect("run always has a shell"),
            &mut self.wl,
        )
    }

    fn flush(&mut self) -> Result<(), String> {
        self.wl.sync();
        self.wl
            .conn
            .flush()
            .map_err(|e| format!("Wayland connection lost: {e}"))
    }
}

pub fn run(shell: Shell, listener: ipc::Listener) -> CmdResult {
    let (mut app, queue) = WlApp::connect(Some(shell)).map_err(|e| {
        (
            exit::NO_SESSION,
            format!("cannot use the Wayland compositor: {e}"),
        )
    })?;
    let event_loop = EventLoop::try_new().map_err(|e| (exit::ERROR, e.to_string()))?;
    WaylandSource::new(app.wl.conn.clone(), queue)
        .insert(event_loop.handle())
        .map_err(|e| (exit::ERROR, e.to_string()))?;
    let (shell, wl) = app.parts();
    shell.start(wl);
    linux::drive(event_loop, &mut app, listener)
}

/// Every output with its scale (for doctor and the GNOME scaling check).
pub fn outputs() -> Result<Vec<DisplayInfo>, String> {
    WlApp::connect(None).map(|(app, _)| app.wl.displays)
}

pub fn doctor(report: &mut Report) -> Vec<DisplayInfo> {
    match WlApp::connect(None) {
        Ok((app, _)) => {
            report.row(
                "primary",
                if !app.wl.order.is_empty() {
                    "first output in kde_output_order_v1"
                } else if app.wl.displays.len() == 1 {
                    "the only output"
                } else {
                    "no primary on this compositor: the first output, or `display = \"<name>\"`"
                },
            );
            app.wl.displays
        }
        Err(e) => {
            report.problem(format!("cannot read Wayland outputs: {e}"));
            Vec::new()
        }
    }
}

impl CompositorHandler for WlApp {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WlApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.wl.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.wl.add_probe(output);
        self.displays_changed();
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.displays_changed();
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.wl.probes.retain(|p| p.output != output);
        self.displays_changed();
    }
}

impl LayerShellHandler for WlApp {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.wl.badge.as_ref().is_some_and(|b| &b.layer == layer) {
            log::info!("wayland: badge surface closed by the compositor; recreating");
            self.wl.badge = None;
            self.wl.new_frame = true;
        }
        self.wl.probes.retain(|p| &p.layer != layer);
        if self.wl.catcher.as_ref().is_some_and(|c| &c.layer == layer) {
            self.wl.catcher = None;
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let wl = &mut self.wl;
        if let Some(badge) = wl.badge.as_mut().filter(|b| &b.layer == layer) {
            badge.configured = true;
            wl.dirty = true;
            if badge.buffer.is_none() {
                wl.new_frame = true;
            }
        } else if wl.catcher.as_ref().is_some_and(|c| &c.layer == layer) {
            wl.fill_catcher(configure.new_size);
        } else if let Some(probe) = wl.probes.iter_mut().find(|p| &p.layer == layer) {
            probe.usable = Some(configure.new_size);
            self.displays_changed();
        }
    }
}

impl SeatHandler for WlApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.wl.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        let wl = &mut self.wl;
        if capability == Capability::Pointer && wl.pointer.is_none() {
            wl.pointer = wl.seats.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && wl.keyboard.is_none() {
            wl.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(p) = self.wl.pointer.take()
        {
            p.release();
        }
        if capability == Capability::Keyboard
            && let Some(k) = self.wl.keyboard.take()
        {
            k.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for WlApp {
    /// Move mode drag, on the catcher: its local coordinates times the
    /// scale, from the usable area's origin, are the core's coordinates.
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            let Some(c) = self
                .wl
                .catcher
                .as_ref()
                .filter(|c| c.layer.wl_surface() == &e.surface)
            else {
                continue;
            };
            let (work, s) = (c.display.work_area, c.display.scale);
            let at = Point::new(
                work.x + (e.position.0 * s).round() as i32,
                work.y + (e.position.1 * s).round() as i32,
            );
            let on_badge = self.wl.rect.is_some_and(|r| r.contains(at));
            let event = match e.kind {
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } if on_badge => {
                    self.wl.dragging = true;
                    Event::DragStart(at)
                }
                PointerEventKind::Motion { .. } if self.wl.dragging => Event::DragMove(at),
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } if self.wl.dragging => {
                    self.wl.dragging = false;
                    Event::DragEnd(at)
                }
                _ => continue,
            };
            self.send(event);
        }
    }
}

/// Move-mode keys by wl_keyboard keycode. Keycodes come from the keymap
/// the compositor sends, read just far enough to find seven keysyms, so
/// virtual keyboards with their own keymaps (remote desktop servers, wtype)
/// work without libxkbcommon. Without a usable keymap, the evdev codes.
fn move_keys(keymap: Option<&str>) -> HashMap<u32, KeyCode> {
    let wanted = |sym: &str| match sym {
        "Left" => Some(KeyCode::Left),
        "Right" => Some(KeyCode::Right),
        "Up" => Some(KeyCode::Up),
        "Down" => Some(KeyCode::Down),
        "Return" | "KP_Enter" => Some(KeyCode::Enter),
        "Escape" => Some(KeyCode::Escape),
        _ => None,
    };
    let parsed = keymap.map(|text| {
        // xkb_keycodes: `<LEFT> = 113;`
        let codes: HashMap<&str, u32> = text
            .lines()
            .filter_map(|l| {
                let (name, num) = l.trim().split_once('=')?;
                let name = name.trim();
                let n = num.trim().trim_end_matches(';').trim().parse().ok()?;
                name.starts_with('<').then_some((name, n))
            })
            .collect();
        // xkb_symbols: `key <LEFT> { [ Left ] };`, possibly over several
        // lines, possibly as `symbols[Group1]= [ Left ]`.
        let mut keys = HashMap::new();
        for chunk in text.split("key <").skip(1) {
            let Some((name, body)) = chunk.split_once('>') else {
                continue;
            };
            let body = body.split("};").next().unwrap_or(body);
            let body = body
                .split_once("symbols[")
                .map_or(body, |(_, b)| b.split_once("]=").map_or(b, |(_, b)| b));
            let first = body
                .split_once('[')
                .and_then(|(_, b)| b.split([',', ']']).next());
            if let Some(code) = first.map(str::trim).and_then(wanted) {
                keys.insert(format!("<{name}>"), code);
            }
        }
        keys.into_iter()
            .filter_map(|(name, code)| Some((codes.get(name.as_str())?.checked_sub(8)?, code)))
            .collect::<HashMap<_, _>>()
    });
    parsed.filter(|m| !m.is_empty()).unwrap_or_else(|| {
        use KeyCode::*;
        HashMap::from([
            (105, Left),
            (106, Right),
            (103, Up),
            (108, Down),
            (28, Enter),
            (96, Enter),
            (1, Escape),
        ])
    })
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WlApp {
    fn event(
        app: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { fd, size, .. } => {
                use std::io::Read;
                let mut text = String::new();
                let _ = std::fs::File::from(fd)
                    .take(u64::from(size))
                    .read_to_string(&mut text);
                app.wl.keys = move_keys(Some(text.trim_end_matches('\0')));
            }
            // Shift is modifier 0 in every XKB keymap.
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                app.wl.shift = mods_depressed & 1 != 0
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } if app.wl.interactive => {
                if let Some(&code) = app.wl.keys.get(&key) {
                    let shift = app.wl.shift;
                    app.send(Event::Key(Key { code, shift }));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<kde_output_order_v1::KdeOutputOrderV1, ()> for WlApp {
    fn event(
        app: &mut Self,
        _: &kde_output_order_v1::KdeOutputOrderV1,
        event: kde_output_order_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            kde_output_order_v1::Event::Output { output_name } => {
                app.wl.order_next.push(output_name)
            }
            kde_output_order_v1::Event::Done => {
                app.wl.order = std::mem::take(&mut app.wl.order_next);
                app.displays_changed();
            }
            _ => {}
        }
    }
}

delegate_noop!(WlApp: ignore wp_viewporter::WpViewporter);
delegate_noop!(WlApp: ignore wp_viewport::WpViewport);

impl ShmHandler for WlApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.wl.shm
    }
}

impl ProvidesRegistryState for WlApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.wl.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(WlApp);
smithay_client_toolkit::delegate_dispatch2!(WlApp);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_keys_from_a_keymap() {
        let keymap = r#"xkb_keymap {
xkb_keycodes "(unnamed)" {
	minimum = 8;
	<ESC> = 9;
	<RTRN> = 36;
	<LEFT> = 113;
	<I10> = 10;
};
xkb_symbols "(unnamed)" {
	key <ESC> { [ Escape ] };
	key <RTRN> { type= "ONE_LEVEL", symbols[Group1]= [ Return ] };
	key <LEFT>               {
		type= "ONE_LEVEL",
		symbols[1]= [            Left ]
	};
	key <I10> { [ a ] };
};
};"#;
        let keys = move_keys(Some(keymap));
        assert_eq!(keys.get(&1), Some(&KeyCode::Escape));
        assert_eq!(keys.get(&28), Some(&KeyCode::Enter));
        assert_eq!(keys.get(&105), Some(&KeyCode::Left));
        assert_eq!(keys.get(&2), None);
        assert_eq!(
            move_keys(Some("garbage")).get(&106),
            Some(&KeyCode::Right),
            "evdev fallback"
        );
    }
}
