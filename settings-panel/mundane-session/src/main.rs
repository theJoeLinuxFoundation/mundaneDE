//! GDM's foreground session process. The compositor and its children inherit the PAM scope.
use anyhow::{bail, ensure, Context, Result};
use gio::{
    glib::{self, ControlFlow, IOCondition, VariantTy},
    prelude::*,
};
use std::{
    cell::RefCell,
    env,
    ffi::OsString,
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::OwnedFd,
        unix::net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    rc::Rc,
    time::{Duration, Instant},
};

const KEYS: &[&str] = &[
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_TYPE",
    "XDG_SESSION_ID",
    "MUNDANE_IBUS_PANEL",
    "DISPLAY",
    "XAUTHORITY",
];
const LOGIN: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const TIMEOUT: i32 = 5000;

fn run(program: &str, args: &[&str]) -> Result<()> {
    ensure!(
        Command::new(program).args(args).status()?.success(),
        "{program} {args:?} failed"
    );
    Ok(())
}
fn publish(clear: bool) -> Result<()> {
    let mut update = Command::new("dbus-update-activation-environment");
    update.arg("--systemd");
    let mut absent = vec!["--user", "unset-environment"];
    for &key in KEYS {
        let mut arg = OsString::from(format!("{key}="));
        if let Some(v) = env::var_os(key).filter(|_| !clear) {
            arg.push(v);
        } else {
            absent.push(key);
        }
        update.arg(arg);
    }
    ensure!(update.status()?.success(), "publish activation environment");
    if absent.len() > 2 {
        run("systemctl", &absent)?;
    }
    Ok(())
}

struct Login {
    bus: gio::DBusConnection,
    session: glib::variant::ObjectPath,
}
impl Login {
    fn connect() -> Result<Self> {
        let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)?;
        let reply = bus.call_sync(
            Some(LOGIN),
            PATH,
            MANAGER,
            "GetSession",
            Some(&(env::var("XDG_SESSION_ID")?,).to_variant()),
            Some(VariantTy::new("(o)")?),
            gio::DBusCallFlags::NONE,
            TIMEOUT,
            gio::Cancellable::NONE,
        )?;
        Ok(Self {
            bus,
            session: reply.child_get(0),
        })
    }
    fn inhibit(&self) -> Result<OwnedFd> {
        let (reply, fds) = self.bus.call_with_unix_fd_list_sync(
            Some(LOGIN),
            PATH,
            MANAGER,
            "Inhibit",
            Some(&("sleep", "mundaneDE", "Release display buffers", "delay").to_variant()),
            Some(VariantTy::new("(h)")?),
            gio::DBusCallFlags::NONE,
            TIMEOUT,
            None::<&gio::UnixFDList>,
            gio::Cancellable::NONE,
        )?;
        let handle = reply.child_get::<glib::variant::Handle>(0);
        Ok(fds
            .context("logind did not return a delay inhibitor")?
            .get(handle.0)?)
    }
    fn locked(&self, yes: bool) -> Result<()> {
        self.bus.call_sync(
            Some(LOGIN),
            self.session.as_str(),
            "org.freedesktop.login1.Session",
            "SetLockedHint",
            Some(&(yes,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            TIMEOUT,
            gio::Cancellable::NONE,
        )?;
        Ok(())
    }
}

fn supervise(channel: &UnixStream, signals: &UnixStream) -> Result<()> {
    let login = Rc::new(Login::connect()?);
    let inhibitor = Rc::new(RefCell::new(Some(login.inhibit()?)));
    let main = glib::MainLoop::new(None, false);
    let errors = Rc::new(RefCell::new(None));
    let fail = {
        let main = main.clone();
        let errors = errors.clone();
        move |e: anyhow::Error| {
            *errors.borrow_mut() = Some(e);
            main.quit();
        }
    };
    let sleep = login.bus.subscribe_to_signal(
        Some(LOGIN),
        Some(MANAGER),
        Some("PrepareForSleep"),
        Some(PATH),
        None,
        gio::DBusSignalFlags::NONE,
        {
            let login = login.clone();
            let inhibitor = inhibitor.clone();
            let mut_channel = channel.try_clone()?;
            let fail = fail.clone();
            move |s| {
                let mut ch = &mut_channel;
                let sleeping = s.parameters.child_get::<bool>(0);
                if let Err(e) = (|| -> Result<()> {
                    if !sleeping && inhibitor.borrow().is_none() {
                        *inhibitor.borrow_mut() = Some(login.inhibit()?);
                    }
                    ch.write_all(if sleeping { b"S" } else { b"W" })?;
                    Ok(())
                })() {
                    fail(e);
                }
            }
        },
    );
    let lock = login.bus.subscribe_to_signal(
        Some(LOGIN),
        Some("org.freedesktop.login1.Session"),
        Some("Lock"),
        Some(login.session.as_str()),
        None,
        gio::DBusSignalFlags::NONE,
        {
            let ch = channel.try_clone()?;
            let fail = fail.clone();
            move |_| {
                if let Err(e) = (&ch).write_all(b"L") {
                    fail(e.into());
                }
            }
        },
    );
    let mut ch = channel.try_clone()?;
    let connection = gio::Socket::from_fd(ch.try_clone()?.into())?;
    let watch = SocketExtManual::create_source(
        &connection,
        IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
        gio::Cancellable::NONE,
        Some("compositor"),
        glib::Priority::DEFAULT,
        {
            let main = main.clone();
            let login = login.clone();
            let inhibitor = inhibitor.clone();
            move |_, _| {
                let mut b = [0];
                match ch.read(&mut b) {
                    Ok(1) if b[0] == b'S' => {
                        inhibitor.borrow_mut().take();
                    }
                    Ok(1) if matches!(b[0], b'L' | b'U') => {
                        if let Err(e) = login.locked(b[0] == b'L') {
                            fail(e);
                        }
                    }
                    _ => main.quit(),
                }
                ControlFlow::Continue
            }
        },
    );
    let terminate = gio::Socket::from_fd(signals.try_clone()?.into())?;
    let stop = main.clone();
    let interrupt = SocketExtManual::create_source(
        &terminate,
        IOCondition::IN,
        gio::Cancellable::NONE,
        Some("termination"),
        glib::Priority::DEFAULT,
        move |_, _| {
            stop.quit();
            ControlFlow::Continue
        },
    );
    watch.attach(None);
    interrupt.attach(None);
    main.run();
    watch.destroy();
    interrupt.destroy();
    drop((lock, sleep));
    let result = errors.borrow_mut().take();
    if let Some(e) = result {
        return Err(e);
    }
    Ok(())
}

fn enter(
    listener: &UnixListener,
    child: &mut Child,
    published: &mut bool,
    signals: &UnixStream,
    connection: &mut Option<UnixStream>,
) -> Result<()> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let accepted = loop {
        let mut signal = [0];
        ensure!(
            (&*signals)
                .read(&mut signal)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock),
            "session interrupted"
        );
        match listener.accept() {
            Ok((channel, _)) => break channel,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
            Err(e) => return Err(e.into()),
        }
        ensure!(
            child.try_wait()?.is_none(),
            "compositor exited before readiness"
        );
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for compositor"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let channel = connection.insert(accepted);
    channel.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut name = String::new();
    BufReader::new((&*channel).take(128)).read_line(&mut name)?;
    let socket = name
        .strip_suffix('\n')
        .context("incomplete compositor readiness message")?;
    ensure!(
        socket
            .strip_prefix("wayland-")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
        "invalid Wayland socket name"
    );
    env::set_var("WAYLAND_DISPLAY", socket);
    *published = true;
    publish(false)?;
    run("systemctl", &["--user", "start", "mundane-session.target"])?;
    channel.write_all(b"1")?;
    run(
        "gdbus",
        &[
            "wait",
            "--session",
            "--timeout=15",
            "org.freedesktop.impl.portal.desktop.mundane",
        ],
    )?;
    run(
        "systemctl",
        &[
            "--user",
            "restart",
            "--no-block",
            "xdg-desktop-portal.service",
        ],
    )?;
    supervise(channel, signals)
}

fn main() -> Result<()> {
    if env::args_os().count() != 1 {
        bail!("usage: mundane-session");
    }
    let bindir = env::current_exe()?
        .parent()
        .context("missing binary directory")?
        .to_owned();
    for key in ["WAYLAND_SOCKET", "WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY"] {
        env::remove_var(key);
    }
    for (k, v) in [
        ("XDG_CURRENT_DESKTOP", "mundane"),
        ("XDG_SESSION_DESKTOP", "mundane"),
        ("XDG_SESSION_TYPE", "wayland"),
        ("LIBSEAT_BACKEND", "logind"),
    ] {
        env::set_var(k, v);
    }
    let temp = tempfile::Builder::new()
        .prefix("mundane-session-")
        .tempdir_in(env::var_os("XDG_RUNTIME_DIR").context("missing runtime directory")?)?;
    let path = temp.path().join("ready");
    let listener = UnixListener::bind(&path)?;
    let mut command = Command::new(bindir.join("mundane-de"));
    command
        .arg("--session-notify")
        .arg(path)
        .env("MUNDANE_PANEL", bindir.join("mundane-shell"));
    let frontend = env::var_os("MUNDANE_IBUS_PANEL")
        .map(PathBuf::from)
        .or_else(|| {
            ["/usr/libexec/ibus-wayland", "/usr/lib/ibus/ibus-wayland"]
                .into_iter()
                .map(Path::new)
                .find(|p| p.is_file())
                .map(Path::to_owned)
        });
    if let Some(frontend) = frontend {
        env::set_var("MUNDANE_IBUS_PANEL", frontend);
    }
    let (signals, tx) = UnixStream::pair()?;
    signals.set_nonblocking(true)?;
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::low_level::pipe::register(signal, tx.try_clone()?)?;
    }
    let mut child = command.spawn()?;
    let mut published = false;
    let mut connection = None;
    let result = enter(
        &listener,
        &mut child,
        &mut published,
        &signals,
        &mut connection,
    );
    if published {
        if let Err(e) = run(
            "systemctl",
            &[
                "--user",
                "stop",
                "mundane-session.target",
                "graphical-session.target",
            ],
        ) {
            eprintln!("{e}");
        }
    }
    if let Some(channel) = connection {
        // GIO can retain a duplicate descriptor until its main context is dispatched.
        // Wake the compositor now so the shell can unregister and exit in order.
        let _ = channel.shutdown(std::net::Shutdown::Both);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while child.try_wait()?.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let status = child.wait()?;
    if published {
        publish(true)?;
    }
    result?;
    ensure!(status.success(), "compositor exited with {status}");
    Ok(())
}
