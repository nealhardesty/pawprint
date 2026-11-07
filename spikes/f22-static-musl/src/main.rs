use smithay_client_toolkit::{
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{globals::registry_queue_init, protocol::wl_output, Connection, QueueHandle};
use x11rb::connection::Connection as _;
use x11rb::protocol::{randr::ConnectionExt as _, xfixes::ConnectionExt as _};

struct App { registry: RegistryState, outputs: OutputState }
impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState { &mut self.outputs }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}
impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry }
    registry_handlers![OutputState];
}
smithay_client_toolkit::delegate_dispatch2!(App);
delegate_registry!(App);

struct Tray;
impl ksni::Tray for Tray {
    fn id(&self) -> String { "spike".into() }
    fn title(&self) -> String { "spike".into() }
}

fn main() {
    // calloop
    let el: calloop::EventLoop<()> = calloop::EventLoop::try_new().expect("calloop");
    println!("calloop: event loop created ({:?})", std::mem::size_of_val(&el));

    // x11rb
    match x11rb::connect(None) {
        Ok((c, screen)) => {
            let root = c.setup().roots[screen].root;
            let xf = c.xfixes_query_version(5, 0).ok().and_then(|r| r.reply().ok());
            let rr = c.randr_get_screen_resources_current(root).ok().and_then(|r| r.reply().ok());
            println!("x11rb: connected, screen {screen}, xfixes={:?}, randr outputs={:?}",
                xf.map(|v| (v.major_version, v.minor_version)), rr.map(|r| r.outputs.len()));
        }
        Err(e) => println!("x11rb: connect failed: {e}"),
    }

    // wayland-client + sctk
    match Connection::connect_to_env() {
        Ok(conn) => match registry_queue_init::<App>(&conn) {
            Ok((globals, mut q)) => {
                let qh = q.handle();
                let mut app = App { registry: RegistryState::new(&globals), outputs: OutputState::new(&globals, &qh) };
                let _ = q.roundtrip(&mut app);
                println!("wayland: connected, {} outputs", app.outputs.outputs().count());
            }
            Err(e) => println!("wayland: registry failed: {e}"),
        },
        Err(e) => println!("wayland: connect failed: {e}"),
    }

    // zbus
    match zbus::blocking::Connection::session() {
        Ok(c) => println!("zbus: session bus ok, unique name {:?}", c.unique_name()),
        Err(e) => println!("zbus: session failed: {e}"),
    }

    // ksni
    use ksni::blocking::TrayMethods;
    match Tray.spawn() {
        Ok(_h) => println!("ksni: tray spawned"),
        Err(e) => println!("ksni: spawn failed: {e}"),
    }
}
