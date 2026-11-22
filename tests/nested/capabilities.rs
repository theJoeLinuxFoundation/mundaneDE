//! Inspect the initial configure as a CSD-capable client would before building its header.
mod presentation;
mod shell_client;
use std::{env, os::unix::net::UnixStream};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_compositor, wl_registry, wl_surface},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::xdg::{
    decoration::zv1::client::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1},
    shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};

#[derive(Default)]
struct Client {
    presented: u64,
    sequence: u64,
    drm: bool,
    discarded: bool,
    serial: Option<u32>,
    capabilities: Vec<u32>,
    client_side: bool,
    server_side_requested: bool,
    requested_size: Option<(i32, i32)>,
    configured: bool,
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
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
delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
delegate_noop!(Client: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Client {
    fn event(
        s: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        e: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(e, xdg_toplevel::Event::Close) && s.server_side_requested {
            println!("close requested");
            std::process::exit(0);
        }
        if let xdg_toplevel::Event::Configure { width, height, .. } = &e {
            s.requested_size = Some((*width, *height));
            println!("configure {width} {height}");
        }
        if let xdg_toplevel::Event::WmCapabilities { capabilities } = e {
            s.capabilities = capabilities
                .chunks_exact(4)
                .map(|v| u32::from_ne_bytes(v.try_into().unwrap()))
                .collect();
            s.capabilities.sort();
        }
    }
}
impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for Client {
    fn event(
        s: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        e: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = e {
            s.client_side = mode == WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ClientSide);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        s: &mut Self,
        x: &xdg_surface::XdgSurface,
        e: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = e {
            assert_eq!(
                s.capabilities,
                [1, 2, 3, 4],
                "window menu, maximize, fullscreen and minimize"
            );
            assert_eq!(s.client_side, !s.server_side_requested);
            x.ack_configure(serial);
            s.configured = true;
        }
    }
}
use wayland_client::protocol::{wl_buffer, wl_keyboard, wl_seat, wl_shm, wl_shm_pool};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
delegate_noop!(Client: ignore wl_shm::WlShm);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_seat::WlSeat);
delegate_noop!(Client: ignore xdg_activation_v1::XdgActivationV1);
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        e: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key: 67,
            state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            serial,
            ..
        } = e
        {
            state.serial = Some(serial);
        }
    }
}
impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for Client {
    fn event(
        _: &mut Self,
        token: &xdg_activation_token_v1::XdgActivationTokenV1,
        e: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token: value } = e {
            println!("launch token {value}");
            token.destroy();
        }
    }
}
fn main() -> anyhow::Result<()> {
    let conn = Connection::from_socket(UnixStream::connect(
        env::args().nth(1).expect("private nested socket"),
    )?)?;
    let (globals, mut queue) = registry_queue_init::<Client>(&conn)?;
    let q = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&q, 1..=5, ())?;
    let wm: xdg_wm_base::XdgWmBase = globals.bind(&q, 5..=6, ())?;
    let manager: zxdg_decoration_manager_v1::ZxdgDecorationManagerV1 =
        globals.bind(&q, 1..=1, ())?;
    let wl = compositor.create_surface(&q, ());
    let surface = wm.get_xdg_surface(&wl, &q, ());
    let top = surface.get_toplevel(&q, ());
    let decoration = manager.get_toplevel_decoration(&top, &q, ());
    let shell = env::args().nth(2).as_deref() == Some("--shell");
    if shell {
        top.set_app_id("org.joelinux.ShellTest".into());
        top.set_title(
            env::args()
                .nth(4)
                .unwrap_or_else(|| "Shell test window".into()),
        );
        decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
    } else {
        decoration.unset_mode();
    }

    wl.commit();
    let mut state = Client {
        server_side_requested: shell,
        ..Client::default()
    };
    while !state.configured {
        queue.blocking_dispatch(&mut state)?;
    }
    if let Some(path) = env::args().nth(2) {
        if shell {
            return shell_client::run(
                &globals,
                &mut queue,
                &mut state,
                &wl,
                std::path::Path::new(&env::args().nth(3).unwrap()),
            );
        }
        if path == "--presentation" {
            state.drm = env::args().nth(3).as_deref() == Some("drm");
            return presentation::exercise(&globals, &mut queue, &mut state, &wl);
        }
        use std::os::fd::AsFd;
        let backing = std::fs::File::create_new(path)?;
        backing.set_len(64 * 64 * 4)?;
        let shm: wl_shm::WlShm = globals.bind(&q, 1..=1, ())?;
        let pool = shm.create_pool(backing.as_fd(), 64 * 64 * 4, &q, ());
        let buffer = pool.create_buffer(0, 64, 64, 256, wl_shm::Format::Xrgb8888, &q, ());
        let seat: wl_seat::WlSeat = globals.bind(&q, 1..=7, ())?;
        let _keyboard = seat.get_keyboard(&q, ());
        let activation: xdg_activation_v1::XdgActivationV1 = globals.bind(&q, 1..=1, ())?;
        wl.attach(Some(&buffer), 0, 0);
        wl.damage(0, 0, 64, 64);
        wl.commit();
        loop {
            queue.blocking_dispatch(&mut state)?;
            if let Some(serial) = state.serial.take() {
                let token = activation.get_activation_token(&q, ());
                token.set_serial(serial, &seat);
                token.set_surface(&wl);
                token.commit();
            }
        }
    }
    Ok(())
}
