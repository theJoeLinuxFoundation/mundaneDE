//! The portal frontend owns requests. Only an explicit GPUI source choice obtains a capture fd.
use crate::{
    cast::{Props, ROOT, STREAM},
    Event,
};
use anyhow::{ensure, Context, Result};
use async_channel::Sender;
use gio::{
    glib::{self, variant::ObjectPath, Variant},
    prelude::*,
};

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    io::Write,
    os::unix::net::{UnixDatagram, UnixStream},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const RECOVERY_PREFIX: &str = "mundane-portal-session-";
const FRONT: &str = "org.freedesktop.portal.Desktop";
const SESSION: &str = "org.freedesktop.impl.portal.Session";
const MARKER: &str = "org.joelinux.mundane.ActiveShare";
const MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
#[derive(Clone, PartialEq)]
pub struct Picker {
    pub serial: u64,
    pub app: String,
    pub types: u32,
    pub share: bool,
}
struct Session {
    reg: gio::RegistrationId,
    types: u32,
    cursor: u32,
    worker: Option<gio::Subprocess>,
    marker: Option<gio::RegistrationId>,
    label: String,
    recovery: Option<tempfile::TempPath>,
}
struct Request {
    path: String,
    reg: gio::RegistrationId,
    reply: gio::DBusMethodInvocation,
    session: Option<String>,
    worker: Option<gio::Subprocess>,
    file: Option<tempfile::TempPath>,
}
#[derive(Default)]
struct State {
    front: String,
    locked: bool,
    serial: u64,
    picker: Option<u64>,
    requests: HashMap<u64, Request>,
    sessions: HashMap<String, Session>,
}
pub struct Portals {
    bus: gio::DBusConnection,
    channel: Arc<UnixDatagram>,
    tx: Sender<Event>,
    state: RefCell<State>,
}
fn info(short: &str) -> Result<gio::DBusInterfaceInfo> {
    let iface = format!("org.freedesktop.impl.portal.{short}");
    let file = format!("dbus-1/interfaces/{iface}.xml");
    let xml = glib::system_data_dirs()
        .iter()
        .find_map(|d| std::fs::read_to_string(d.join(&file)).ok())
        .with_context(|| format!("Missing {file}"))?;
    Ok(gio::DBusNodeInfo::for_xml(&xml)?
        .lookup_interface(&iface)
        .unwrap())
}
fn xml(source: &str) -> Result<gio::DBusInterfaceInfo> {
    Ok(gio::DBusNodeInfo::for_xml(source)?.interfaces()[0].clone())
}
fn reply(inv: gio::DBusMethodInvocation, code: u32, props: Props) {
    inv.return_value(Some(&(code, props).to_variant()));
}
fn stop(p: &gio::Subprocess) {
    if let Some(pid) = p.identifier().and_then(|s| s.parse::<i32>().ok()) {
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
    }
}
impl Portals {
    pub fn new(tx: Sender<Event>, channel: Arc<UnixDatagram>) -> Result<Rc<Self>> {
        let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let p = Rc::new(Self {
            bus: bus.clone(),
            channel,
            tx,
            state: RefCell::new(State {
                locked: true,
                ..Default::default()
            }),
        });
        for name in ["ScreenCast", "Screenshot"] {
            p.register(ROOT, &info(name)?)?;
        }
        let manager = xml(
            r#"<node><interface name="org.freedesktop.DBus.ObjectManager"><method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method><signal name="InterfacesAdded"><arg type="o"/><arg type="a{sa{sv}}"/></signal><signal name="InterfacesRemoved"><arg type="o"/><arg type="as"/></signal></interface></node>"#,
        )?;
        p.register(ROOT, &manager)?;
        let a = p.clone();
        let b = p.clone();
        gio::bus_watch_name_on_connection(
            &bus,
            FRONT,
            gio::BusNameWatcherFlags::NONE,
            move |_, _, owner| {
                a.state.borrow_mut().front = owner.into();
            },
            move |_, _| {
                b.state.borrow_mut().front.clear();
                b.cancel_all();
            },
        );
        Ok(p)
    }
    // The compositor revokes the capture fd immediately on shell loss. Close the
    // corresponding frontend session too, once its backend proxy can hear us again.
    pub fn recover_closed(&self) {
        let Ok(files) = std::fs::read_dir(glib::user_runtime_dir()) else {
            return;
        };
        let owner = self
            .bus
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                Some(&(FRONT,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            )
            .ok()
            .map(|v| v.child_get::<String>(0));
        for file in files.flatten().filter(|f| {
            f.file_name()
                .as_encoded_bytes()
                .starts_with(RECOVERY_PREFIX.as_bytes())
        }) {
            let Ok(bytes) = std::fs::read(file.path()) else {
                continue;
            };
            if let Ok((dest, path)) = serde_json::from_slice::<(String, String)>(&bytes) {
                if self.state.borrow().sessions.contains_key(&path) {
                    continue;
                }
                if owner.as_ref() == Some(&dest)
                    && path.starts_with("/org/freedesktop/portal/desktop/session/")
                    && ObjectPath::try_from(path.as_str()).is_ok()
                {
                    let _ = self.bus.emit_signal(
                        Some(&dest),
                        &path,
                        SESSION,
                        "Closed",
                        Some(&().to_variant()),
                    );
                }
            }
            let _ = std::fs::remove_file(file.path());
        }
    }
    fn register(
        self: &Rc<Self>,
        path: &str,
        info: &gio::DBusInterfaceInfo,
    ) -> Result<gio::RegistrationId> {
        let p = self.clone();
        Ok(self
            .bus
            .register_object(path, info)
            .method_call(move |_, sender, path, iface, m, args, inv| {
                p.method(
                    sender.unwrap_or(""),
                    path,
                    iface.unwrap_or(""),
                    m,
                    args,
                    inv,
                )
            })
            .property(|_, _, _, iface, key| {
                match key {
                    "AvailableSourceTypes" | "AvailableCursorModes" | "AvailableTargets" => 3u32,
                    "version" if iface == STREAM => 3,
                    "version" if iface.ends_with(".Screenshot") => 2,
                    _ => 1,
                }
                .to_variant()
            })
            .build()?)
    }
    fn marker_props(label: &str) -> BTreeMap<String, Props> {
        BTreeMap::from([(
            MARKER.into(),
            Props::from([("Source".into(), label.to_variant())]),
        )])
    }
    fn signal(&self, path: &str, iface: &str, signal: &str, args: Variant) {
        let _ = self.bus.emit_signal(None, path, iface, signal, Some(&args));
    }
    fn method(
        self: &Rc<Self>,
        sender: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: Variant,
        inv: gio::DBusMethodInvocation,
    ) {
        if iface == MANAGER && method == "GetManagedObjects" {
            let map: BTreeMap<ObjectPath, _> = self
                .state
                .borrow()
                .sessions
                .iter()
                .filter(|(_, s)| s.marker.is_some())
                .map(|(p, s)| {
                    (
                        ObjectPath::try_from(p.as_str()).unwrap(),
                        Self::marker_props(&s.label),
                    )
                })
                .collect();
            inv.return_value(Some(&(map,).to_variant()));
            return;
        }
        if sender.is_empty() || sender != self.state.borrow().front {
            inv.return_dbus_error(
                "org.freedesktop.DBus.Error.AccessDenied",
                "Portal frontend required",
            );
            return;
        }
        if method == "Close" {
            if iface == SESSION {
                self.close(path);
            } else {
                let id = self
                    .state
                    .borrow()
                    .requests
                    .iter()
                    .find(|(_, r)| r.path == path)
                    .map(|(id, _)| *id);
                if let Some(id) = id {
                    self.finish(id, 1, Props::new());
                }
            }
            inv.return_value(None);
            return;
        }
        if self.state.borrow().locked {
            reply(inv, 1, Props::new());
            return;
        }
        let obj = |i| args.child_get::<ObjectPath>(i).to_string();
        match method {
            "CreateSession" => {
                let path = obj(1);
                let reg = info("Session").and_then(|i| self.register(&path, &i));
                if let Ok(reg) = reg {
                    self.state.borrow_mut().sessions.insert(
                        path,
                        Session {
                            reg,
                            types: 0,
                            cursor: 1,
                            worker: None,
                            marker: None,
                            label: String::new(),
                            recovery: None,
                        },
                    );
                    reply(inv, 0, Props::new());
                } else {
                    reply(inv, 2, Props::new());
                }
            }
            "SelectSources" => {
                let options: Props = args.child_get(3);
                let uint = |k: &str| options.get(k).and_then(|v| v.get::<u32>()).unwrap_or(1);
                let mut st = self.state.borrow_mut();
                let code = if let Some(s) = st.sessions.get_mut(&obj(1)) {
                    if s.types == 0
                        && uint("types") & 3 != 0
                        && matches!(uint("cursor_mode"), 1 | 2)
                    {
                        s.types = uint("types") & 3;
                        s.cursor = uint("cursor_mode");
                        0
                    } else {
                        2
                    }
                } else {
                    2
                };
                reply(inv, code, Props::new());
            }
            "Start" | "Screenshot" => {
                let share = method == "Start";
                let path = obj(0);
                let session = share.then(|| obj(1));
                let mut st = self.state.borrow_mut();
                if st.picker.is_some() {
                    reply(inv, 1, Props::new());
                    return;
                }
                let types = if let Some(p) = &session {
                    match st.sessions.get(p) {
                        Some(s)
                            if s.types != 0
                                && s.worker.is_none()
                                && !st.requests.values().any(|r| r.session.as_ref() == Some(p)) =>
                        {
                            s.types
                        }
                        _ => {
                            reply(inv, 2, Props::new());
                            return;
                        }
                    }
                } else {
                    3
                };
                let Ok(reg) = info("Request").and_then(|i| self.register(&path, &i)) else {
                    reply(inv, 2, Props::new());
                    return;
                };
                st.serial += 1;
                let serial = st.serial;
                st.requests.insert(
                    serial,
                    Request {
                        path,
                        reg,
                        reply: inv,
                        session,
                        worker: None,
                        file: None,
                    },
                );
                st.picker = Some(serial);
                let _ = self.tx.try_send(Event::Picker(Picker {
                    serial,
                    app: args.child_get(if share { 2 } else { 1 }),
                    types,
                    share,
                }));
            }
            _ => reply(inv, 2, Props::new()),
        }
    }
    pub fn locked(&self, v: bool) {
        self.state.borrow_mut().locked = v;
        if v {
            self.cancel_all();
        }
    }
    fn cancel_all(&self) {
        let ids: Vec<_> = self.state.borrow().requests.keys().copied().collect();
        for id in ids {
            self.finish(id, 1, Props::new());
        }
        let paths: Vec<_> = self.state.borrow().sessions.keys().cloned().collect();
        for p in paths {
            self.close(&p);
        }
    }
    pub fn cancel(&self, id: u64) {
        self.finish(id, 1, Props::new());
    }
    fn finish(&self, id: u64, code: u32, props: Props) {
        let mut st = self.state.borrow_mut();
        let Some(r) = st.requests.remove(&id) else {
            return;
        };
        if st.picker == Some(id) {
            st.picker = None;
            let _ = self.tx.try_send(Event::PickerClosed(id));
        }
        drop(st);
        let _ = self.bus.unregister_object(r.reg);
        if code != 0 {
            if let Some(p) = &r.worker {
                stop(p);
            }
        }
        reply(r.reply, code, props);
    }
    pub fn close(&self, path: &str) {
        let mut st = self.state.borrow_mut();
        let Some(s) = st.sessions.remove(path) else {
            return;
        };
        let ids: Vec<_> = st
            .requests
            .iter()
            .filter(|(_, r)| r.session.as_deref() == Some(path))
            .map(|(id, _)| *id)
            .collect();
        drop(st);
        for id in ids {
            self.finish(id, 1, Props::new());
        }
        if let Some(p) = s.worker {
            stop(&p);
        }
        if let Some(reg) = s.marker {
            let _ = self.bus.unregister_object(reg);
            self.signal(
                ROOT,
                MANAGER,
                "InterfacesRemoved",
                (ObjectPath::try_from(path).unwrap(), vec![MARKER]).to_variant(),
            );
            self.publish();
        }
        let _ = self.bus.unregister_object(s.reg);
        self.signal(path, SESSION, "Closed", ().to_variant());
    }
    fn publish(&self) {
        let mut active: Vec<_> = self
            .state
            .borrow()
            .sessions
            .iter()
            .filter(|(_, s)| s.marker.is_some())
            .map(|(path, s)| (path.clone(), s.label.clone()))
            .collect();
        active.sort();
        let _ = self.tx.try_send(Event::Shares(active));
    }
    fn grant(&self, op: &str, id: &str) -> Result<UnixStream> {
        let (server, client) = UnixStream::pair()?;
        use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags};
        use std::{io::IoSlice, os::fd::AsRawFd};
        let cmd = format!("{op} {id}");
        sendmsg::<()>(
            self.channel.as_raw_fd(),
            &[IoSlice::new(cmd.as_bytes())],
            &[ControlMessage::ScmRights(&[server.as_raw_fd()])],
            MsgFlags::MSG_NOSIGNAL,
            None,
        )?;
        Ok(client)
    }
    pub fn accept(self: &Rc<Self>, serial: u64, kind: &str, id: &str, label: String) {
        if let Err(e) = self.start(serial, kind, id, label) {
            eprintln!("Capture: {e:#}");
            self.finish(serial, 2, Props::new());
        }
    }
    fn start(self: &Rc<Self>, serial: u64, kind: &str, id: &str, label: String) -> Result<()> {
        let mut st = self.state.borrow_mut();
        ensure!(
            !st.locked && st.picker == Some(serial) && matches!(kind, "output" | "window"),
            "Consent expired"
        );
        st.picker = None;
        let session = st
            .requests
            .get(&serial)
            .context("Request closed")?
            .session
            .clone();
        let op = match (kind, session.is_some()) {
            ("output", true) => "capture-output",
            ("output", false) => "screenshot-output",
            _ => "capture-window",
        };
        let fd = self.grant(op, id)?;
        let launch = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE);
        launch.set_child_setup(|| unsafe {
            libc::setpgid(0, 0);
        });
        launch.setenv("WAYLAND_SOCKET", "0", true);
        launch.take_stdin_fd(Some(fd.into()));
        let p = if let Some(path) = &session {
            let s = st.sessions.get_mut(path).context("Session closed")?;
            let exe = std::env::current_exe()?;
            let p = launch.spawn(&[
                OsStr::new("dbus-run-session"),
                "--".as_ref(),
                exe.as_os_str(),
                "--cast-worker".as_ref(),
                kind.as_ref(),
                id.as_ref(),
                s.cursor.to_string().as_ref(),
            ])?;
            s.worker = Some(p.clone());
            s.label = label;
            p
        } else {
            let file = tempfile::Builder::new()
                .prefix("mundane-screenshot-")
                .suffix(".png")
                .tempfile_in(glib::user_runtime_dir())?
                .into_temp_path();
            let p = launch.spawn(&[
                "grim".as_ref(),
                if kind == "output" { "-o" } else { "-T" }.as_ref(),
                id.as_ref(),
                file.as_os_str(),
            ])?;
            st.requests.get_mut(&serial).unwrap().file = Some(file);
            p
        };
        st.requests.get_mut(&serial).unwrap().worker = Some(p.clone());
        drop(st);
        let ctx = glib::MainContext::ref_thread_default();
        if let Some(path) = session.clone() {
            let out = gio::DataInputStream::new(&p.stdout_pipe().unwrap());
            let portal = self.clone();
            ctx.spawn_local(async move {
                let result = async {
                    let line = out
                        .read_line_utf8_future(glib::Priority::DEFAULT)
                        .await?
                        .context("No PipeWire node")?;
                    let streams = Variant::parse(Some(glib::VariantTy::new("a(ua{sv})")?), &line)?;
                    ensure!(
                        streams.n_children() == 1
                            && matches!(streams.child_value(0).child_get::<u32>(0), 1..=4294967294),
                        "Invalid PipeWire node"
                    );
                    portal.ready(serial, &path, streams)
                }
                .await;
                if let Err(e) = result {
                    eprintln!("Screencast startup: {e:#}");
                    portal.finish(serial, 2, Props::new());
                }
            });
        }
        let portal = self.clone();
        ctx.spawn_local(async move {
            let success = p.wait_check_future().await.is_ok();
            if let Some(path) = session {
                if portal
                    .state
                    .borrow()
                    .sessions
                    .get(&path)
                    .is_some_and(|s| s.worker.as_ref() == Some(&p))
                {
                    portal.close(&path);
                }
            } else {
                let file = portal
                    .state
                    .borrow_mut()
                    .requests
                    .get_mut(&serial)
                    .and_then(|r| r.file.take());
                let uri = file
                    .filter(|f| success && f.metadata().is_ok_and(|m| m.len() > 0))
                    .and_then(|f| f.keep().ok())
                    .map(|p| gio::File::for_path(p).uri());
                if let Some(uri) = uri {
                    portal.finish(serial, 0, Props::from([("uri".into(), uri.to_variant())]));
                } else {
                    portal.finish(serial, 2, Props::new());
                }
            }
        });
        let portal = self.clone();
        ctx.spawn_local(async move {
            glib::timeout_future(Duration::from_secs(20)).await;
            portal.finish(serial, 2, Props::new());
        });
        Ok(())
    }
    fn ready(&self, id: u64, path: &str, streams: Variant) -> Result<()> {
        let mut st = self.state.borrow_mut();
        ensure!(st.requests.contains_key(&id), "Request cancelled");
        let mut recovery = tempfile::Builder::new()
            .prefix(RECOVERY_PREFIX)
            .tempfile_in(glib::user_runtime_dir())?;
        serde_json::to_writer(&mut recovery, &(&st.front, path))?;
        recovery.flush()?;
        let s = st.sessions.get_mut(path).context("Session closed")?;
        let label = s.label.clone();
        let info = xml(
            r#"<node><interface name="org.joelinux.mundane.ActiveShare"><property name="Source" type="s" access="read"/></interface></node>"#,
        )?;
        s.marker = Some(
            self.bus
                .register_object(path, &info)
                .property(move |_, _, _, _, _| label.to_variant())
                .build()?,
        );
        s.recovery = Some(recovery.into_temp_path());
        let props = Self::marker_props(&s.label);
        drop(st);
        self.signal(
            ROOT,
            MANAGER,
            "InterfacesAdded",
            (ObjectPath::try_from(path)?, props).to_variant(),
        );
        self.publish();
        self.finish(id, 0, Props::from([("streams".into(), streams)]));
        Ok(())
    }
}
