//! A normal Wayland client with a visible idle inhibitor and a negotiated server title bar.
use super::Client;
use std::{
    io::Write,
    os::{
        fd::AsFd,
        unix::net::{UnixListener, UnixStream},
    },
    path::Path,
};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalList},
    protocol::{wl_seat, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::{
    ext::idle_notify::v1::client::{
        ext_idle_notification_v1 as notification, ext_idle_notifier_v1 as idle,
    },
    wp::{
        idle_inhibit::zv1::client::{
            zwp_idle_inhibit_manager_v1 as inhibit, zwp_idle_inhibitor_v1 as inhibitor,
        },
        security_context::v1::client::{
            wp_security_context_manager_v1 as manager, wp_security_context_v1 as context,
        },
    },
};
delegate_noop!(Client: ignore inhibit::ZwpIdleInhibitManagerV1);
delegate_noop!(Client: ignore inhibitor::ZwpIdleInhibitorV1);
delegate_noop!(Client: ignore manager::WpSecurityContextManagerV1);
delegate_noop!(Client: ignore context::WpSecurityContextV1);
delegate_noop!(Client: ignore idle::ExtIdleNotifierV1);
impl Dispatch<notification::ExtIdleNotificationV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &notification::ExtIdleNotificationV1,
        event: notification::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        println!("idle event {event:?}");
    }
}
pub fn run(
    globals: &GlobalList,
    queue: &mut wayland_client::EventQueue<Client>,
    state: &mut Client,
    wl: &wl_surface::WlSurface,
    dir: &Path,
) -> anyhow::Result<()> {
    let q = queue.handle();
    let sock = dir.join("sandbox");
    let listener = UnixListener::bind(&sock)?;
    let (lifetime, _keepalive) = UnixStream::pair()?;
    let manager: manager::WpSecurityContextManagerV1 = globals.bind(&q, 1..=1, ())?;
    let ctx = manager.create_listener(listener.as_fd(), lifetime.as_fd(), &q, ());
    ctx.set_sandbox_engine("test".into());
    ctx.set_app_id("org.joelinux.ShellTest".into());
    ctx.commit();
    queue.roundtrip(state)?;
    let sandbox = Connection::from_socket(UnixStream::connect(&sock)?)?;
    let (sg, _) = registry_queue_init::<Client>(&sandbox)?;
    for g in [globals, &sg] {
        let names = g
            .contents()
            .clone_list()
            .into_iter()
            .map(|g| g.interface)
            .collect::<Vec<_>>();
        for hidden in [
            "ext_session_lock_manager_v1",
            "zwlr_layer_shell_v1",
            "zwp_input_method_manager_v2",
            "ext_data_control_manager_v1",
            "zwlr_screencopy_manager_v1",
        ] {
            assert!(!names.iter().any(|n| n == hidden));
        }
        assert_eq!(
            names.iter().any(|n| n == "wp_security_context_manager_v1"),
            std::ptr::eq(g, globals)
        );
        assert!(names.iter().any(|n| n == "ext_foreign_toplevel_list_v1"));
    }
    println!("ordinary and sandbox registries passed");
    let shm: wl_shm::WlShm = globals.bind(&q, 1..=1, ())?;
    let paint = |w: i32, h: i32| -> anyhow::Result<()> {
        let path = dir.join("buffer");
        let mut backing = std::fs::File::create_new(&path)?;
        std::fs::remove_file(path)?;
        backing.write_all(&[0x44, 0xaa, 0xbb, 0xff].repeat((w * h) as usize))?;
        let pool = shm.create_pool(backing.as_fd(), w * h * 4, &q, ());
        let b = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Xrgb8888, &q, ());
        wl.attach(Some(&b), 0, 0);
        wl.damage(0, 0, w, h);
        wl.commit();
        pool.destroy();
        b.destroy();
        Ok(())
    };
    state.requested_size = None;
    paint(320, 170)?;
    let seat: wl_seat::WlSeat = globals.bind(&q, 1..=7, ())?;
    let inhibit: inhibit::ZwpIdleInhibitManagerV1 = globals.bind(&q, 1..=1, ())?;
    let _inhibitor = inhibit.create_inhibitor(wl, &q, ());
    let idle: idle::ExtIdleNotifierV1 = globals.bind(&q, 1..=2, ())?;
    let _notify = idle.get_idle_notification(1900, &seat, &q, ());
    queue.roundtrip(state)?;
    println!("server-decorated client mapped");
    loop {
        queue.blocking_dispatch(state)?;
        if let Some((w, h)) = state.requested_size.take() {
            if w > 0 && h > 0 {
                paint(w, h)?;
            } else {
                wl.commit();
            }
        }
    }
}
