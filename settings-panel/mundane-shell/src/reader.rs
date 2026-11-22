//! Authorize one unique D-Bus connection from the Orca process this shell started.
use anyhow::{Context, Result};
use gio::{glib, prelude::*};
use serde::Deserialize;
use serde_json::json;
use std::{
    cell::{Cell, OnceCell, RefCell},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{net::UnixDatagram, process::CommandExt},
    },
    process::Stdio,
    rc::Rc,
    sync::Arc,
    time::Duration,
};
const NAME: &str = "org.freedesktop.a11y.Manager";
const PATH: &str = "/org/freedesktop/a11y/Manager";
const IFACE: &str = "org.freedesktop.a11y.KeyboardMonitor";
const XML: &str = r#"<node><interface name="org.freedesktop.a11y.KeyboardMonitor">
<method name="WatchKeyboard"/><method name="UnwatchKeyboard"/>
<method name="GrabKeyboard"/><method name="UngrabKeyboard"/>
<method name="SetKeyGrabs"><arg type="au"/><arg type="a(uu)"/></method>
<signal name="KeyEvent"><arg type="b"/><arg type="u"/><arg type="u"/><arg type="u"/><arg type="q"/></signal>
</interface></node>"#;
#[derive(Clone, Deserialize)]
pub struct Key {
    subscription: String,
    serial: u32,
    key: (bool, u32, u32, u32, u16),
}
#[derive(Default)]
struct Watch {
    owner: String,
    subscription: String,
    watching: bool,
    grab: bool,
    modifiers: Vec<u32>,
    keys: Vec<(u32, u32)>,
}
struct Child {
    pid: u32,
    fd: OwnedFd,
}
impl Child {
    fn alive(&self) -> bool {
        let mut poll = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut poll, 1, 0) == 0 }
    }
    fn terminate(&self) {
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
}
pub struct Reader {
    bus: gio::DBusConnection,
    prefs: gio::Settings,
    owner_changes: OnceCell<gio::SignalSubscription>,
    socket: Arc<UnixDatagram>,
    child: RefCell<Option<Child>>,
    watch: RefCell<Watch>,
    acquired: Cell<bool>,
    sensitive: Cell<bool>,
}
impl Reader {
    pub fn new(socket: Arc<UnixDatagram>) -> Result<Rc<Self>> {
        let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let r = Rc::new(Self {
            bus: bus.clone(),
            owner_changes: OnceCell::new(),
            prefs: gio::Settings::new("org.gnome.desktop.a11y.applications"),
            socket,
            child: Default::default(),
            watch: Default::default(),
            acquired: Cell::new(false),
            sensitive: Cell::new(false),
        });
        let reader = r.clone();
        bus.register_object(PATH, &gio::DBusNodeInfo::for_xml(XML)?.interfaces()[0])
            .method_call(move |_, sender, _, _, method, args, reply| {
                let sender = sender.unwrap_or("");
                if !reader.authorized(sender) {
                    reply.return_dbus_error(
                        "org.freedesktop.DBus.Error.AccessDenied",
                        "The session screen reader is required",
                    );
                    return;
                }
                let mut w = reader.watch.borrow_mut();
                w.owner = sender.into();
                match method {
                    "SetKeyGrabs" => (w.modifiers, w.keys) = args.get().unwrap(),
                    "WatchKeyboard" => w.watching = true,
                    "UnwatchKeyboard" => w.watching = false,
                    "GrabKeyboard" => w.grab = true,
                    "UngrabKeyboard" => w.grab = false,
                    _ => unreachable!(),
                }
                drop(w);
                reader.configure();
                reply.return_value(None);
            })
            .build()?;
        let reader = r.clone();
        // The subscription lives for the life of this service.
        let names = bus.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let (name, _, new) = signal.parameters.get::<(String, String, String)>().unwrap();
                if new.is_empty() && name == reader.watch.borrow().owner {
                    reader.clear()
                }
            },
        );
        r.owner_changes.set(names).ok();
        let reader = r.clone();
        r.prefs
            .connect_changed(Some("screen-reader-enabled"), move |_, _| {
                reader.reconcile()
            });
        let a = r.clone();
        let b = r.clone();
        gio::bus_own_name_on_connection(
            &bus,
            NAME,
            gio::BusNameOwnerFlags::NONE,
            move |_, _| {
                a.acquired.set(true);
                a.reconcile()
            },
            move |_, _| {
                b.acquired.set(false);
                b.reconcile()
            },
        );
        Ok(r)
    }
    fn authorized(&self, sender: &str) -> bool {
        let w = self.watch.borrow();
        if sender.is_empty()
            || !self.acquired.get()
            || !self.prefs.boolean("screen-reader-enabled")
            || (!w.owner.is_empty() && w.owner != sender)
        {
            return false;
        }
        let child = self.child.borrow();
        let Some(child) = child.as_ref() else {
            return false;
        };
        if !w.owner.is_empty() {
            return child.alive();
        }
        let pid = self.bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            Some(&(sender,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        );
        pid.is_ok_and(|v| v.child_get::<u32>(0) == child.pid) && child.alive()
    }
    pub fn toggle(&self) {
        let _ = self.prefs.set_boolean(
            "screen-reader-enabled",
            !self.prefs.boolean("screen-reader-enabled"),
        );
    }
    fn reconcile(self: &Rc<Self>) {
        if !self.acquired.get() || !self.prefs.boolean("screen-reader-enabled") {
            self.clear();
            if let Some(child) = self.child.borrow().as_ref() {
                child.terminate()
            }
        } else if self.child.borrow().is_none() {
            if let Err(e) = self.start() {
                eprintln!("Start screen reader: {e:#}")
            }
        }
    }
    fn start(self: &Rc<Self>) -> Result<()> {
        let parent = std::process::id();
        let mut cmd = crate::renderer::command("/usr/bin/orca");
        cmd.arg("--replace")
            .env_remove("WAYLAND_SOCKET")
            .stdin(Stdio::null());
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0
                    || libc::getppid() != parent as i32
                {
                    return Err(std::io::Error::other(
                        "screen reader supervisor unavailable",
                    ));
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        // Keep the identity alive across PID reuse. Only GLib reaps this child.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("pin Orca process");
        }
        self.child.replace(Some(Child {
            pid,
            fd: unsafe { OwnedFd::from_raw_fd(fd as i32) },
        }));
        let r = self.clone();
        glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::child_watch_future(glib::Pid(pid as i32)).await;
            r.child.take();
            r.clear();
            glib::timeout_future(Duration::from_secs(1)).await;
            r.reconcile();
        });
        Ok(())
    }
    pub fn sensitive(&self, value: bool) {
        if self.sensitive.replace(value) != value {
            self.configure()
        }
    }
    fn clear(&self) {
        self.watch.replace(Watch::default());
        self.configure();
    }
    fn configure(&self) {
        let mut w = self.watch.borrow_mut();
        let enabled = !self.sensitive.get()
            && !w.owner.is_empty()
            && (w.watching || w.grab || !w.modifiers.is_empty() || !w.keys.is_empty());
        let config = if enabled {
            if w.subscription.is_empty() {
                w.subscription = glib::uuid_string_random().to_string()
            }
            json!({"subscription":w.subscription,"all":w.grab,"modifiers":w.modifiers,"keys":w.keys})
        } else {
            w.subscription.clear();
            json!({})
        };
        let _ = self
            .socket
            .send(format!("keyboard-monitor {config}").as_bytes());
    }
    pub fn key(&self, event: Key) {
        let w = self.watch.borrow();
        let socket = self.socket.clone();
        let forward = move || {
            let _ = socket.send(format!("keyboard-forward {}", event.serial).as_bytes());
        };
        if !w.owner.is_empty()
            && !w.subscription.is_empty()
            && w.subscription == event.subscription
            && !self.sensitive.get()
        {
            let _ = self.bus.emit_signal(
                Some(&w.owner),
                PATH,
                IFACE,
                "KeyEvent",
                Some(&event.key.to_variant()),
            );
            // Order the physical key before the corresponding app edit on AT-SPI.
            self.bus.call(
                Some(&w.owner),
                PATH,
                "org.freedesktop.DBus.Peer",
                "Ping",
                None,
                None,
                gio::DBusCallFlags::NONE,
                40,
                gio::Cancellable::NONE,
                move |_| forward(),
            );
        } else {
            forward()
        }
    }
}
