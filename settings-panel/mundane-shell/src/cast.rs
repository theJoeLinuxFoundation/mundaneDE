//! One private xdpw instance for the source already granted on standard input.
use anyhow::{ensure, Context, Result};
use gio::{
    glib::{self, variant::ObjectPath, Variant},
    prelude::*,
};
use std::{
    collections::BTreeMap,
    io::{Seek, Write},
    os::fd::{AsFd, AsRawFd},
    time::Duration,
};
const WLR: &str = "org.freedesktop.impl.portal.desktop.wlr";
pub const ROOT: &str = "/org/freedesktop/portal/desktop";
pub const STREAM: &str = "org.freedesktop.impl.portal.ScreenCast";
pub type Props = BTreeMap<String, Variant>;
pub fn worker(kind: &str, id: &str, cursor: &str) -> Result<()> {
    ensure!(matches!(kind, "output" | "window"), "Unknown capture type");
    let mut config = tempfile::tempfile_in(glib::user_runtime_dir())?;
    config.write_all(b"[screencast]\nchooser_type=simple\nchooser_cmd=printf '%s\\n' \"$MUNDANE_SELECTED_SOURCE\"\nmax_fps=30\n")?;
    config.rewind()?;
    let exe = glib::find_program_in_path("xdg-desktop-portal-wlr")
        .or_else(|| {
            [
                "/usr/libexec/xdg-desktop-portal-wlr",
                "/usr/lib/xdg-desktop-portal-wlr",
            ]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.is_file())
        })
        .context("xdg-desktop-portal-wlr is unavailable")?;
    let launch = crate::renderer::launcher(gio::SubprocessFlags::NONE);
    launch.take_stdin_fd(Some(std::io::stdin().as_fd().try_clone_to_owned()?));
    launch.take_stdout_fd(Some(std::io::stderr().as_fd().try_clone_to_owned()?));
    launch.take_fd(config.as_fd().try_clone_to_owned()?, &config);
    launch.setenv("WAYLAND_SOCKET", "0", true);
    let source = if kind == "output" {
        "Monitor"
    } else {
        "Window"
    };
    launch.setenv("MUNDANE_SELECTED_SOURCE", format!("{source}: {id}"), true);
    let config_path = format!("/proc/self/fd/{}", config.as_raw_fd());
    let wlr = launch.spawn(&[exe.as_os_str(), "-c".as_ref(), config_path.as_ref()])?;
    let run = (|| -> Result<()> {
        let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        for _ in 0..100 {
            ensure!(wlr.identifier().is_some(), "xdpw exited before startup");
            let owner = bus.call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&(WLR,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            )?;
            if owner.child_get::<bool>(0) {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let session = ObjectPath::try_from(format!("{ROOT}/session/mundane/cast"))?;
        let handle = |s: &str| ObjectPath::try_from(format!("{ROOT}/request/mundane/{s}")).unwrap();
        let call = |method: &str, args: Variant| -> Result<Props> {
            let r = bus.call_sync(
                Some(WLR),
                ROOT,
                STREAM,
                method,
                Some(&args),
                None,
                gio::DBusCallFlags::NONE,
                12000,
                gio::Cancellable::NONE,
            )?;
            ensure!(r.child_get::<u32>(0) == 0, "xdpw declined {method}");
            Ok(r.child_get(1))
        };
        call(
            "CreateSession",
            (handle("create"), &session, "", Props::new()).to_variant(),
        )?;
        let options = Props::from([
            (
                "types".into(),
                (if kind == "output" { 1u32 } else { 2 }).to_variant(),
            ),
            ("cursor_mode".into(), cursor.parse::<u32>()?.to_variant()),
        ]);
        call(
            "SelectSources",
            (handle("select"), &session, "", options).to_variant(),
        )?;
        let main = glib::MainLoop::new(None, false);
        let quit = main.clone();
        let _closed = bus.subscribe_to_signal(
            Some(WLR),
            Some("org.freedesktop.impl.portal.Session"),
            Some("Closed"),
            Some(session.as_str()),
            None,
            gio::DBusSignalFlags::NONE,
            move |_| quit.quit(),
        );
        let result = call(
            "Start",
            (handle("start"), &session, "", "", Props::new()).to_variant(),
        )?;
        let streams = result
            .get("streams")
            .context("xdpw did not provide a stream")?;
        println!("{}", streams.print(true));
        std::io::stdout().flush()?;
        // Child exit and the private session's Closed signal independently revoke the stream.
        let quit = main.clone();
        wlr.wait_async(gio::Cancellable::NONE, move |_| quit.quit());
        main.run();
        Ok(())
    })();
    if wlr.identifier().is_some() {
        wlr.force_exit();
    }
    let _ = wlr.wait(gio::Cancellable::NONE);
    run
}
