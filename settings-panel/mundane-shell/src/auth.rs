//! System-bus polkit agent. Only polkitd can begin or withdraw a conversation.
use crate::{reader::Reader, Event};
use async_channel::Sender;
use gio::{
    glib::{self, translate::from_glib_full},
    prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ffi::CString,
    rc::Rc,
};
use zeroize::Zeroizing;

const AUTH: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1.Authority";
const PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AGENT: &str = "/org/joelinux/mundane/AuthenticationAgent";
const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const XML: &str = r#"<node><interface name="org.freedesktop.PolicyKit1.AuthenticationAgent">
<method name="BeginAuthentication"><arg type="s"/><arg type="s"/><arg type="s"/><arg type="a{ss}"/><arg type="s"/><arg type="a(sa{sv})"/></method>
<method name="CancelAuthentication"><arg type="s"/></method></interface></node>"#;

// Both libraries use GObjects. Keep a strong reference across each call since
// initiate, response and cancel may synchronously emit completed.
type GObject = glib::gobject_ffi::GObject;
#[link(name = "polkit-gobject-1")]
unsafe extern "C" {
    fn polkit_unix_user_new(uid: libc::c_int) -> *mut GObject;
}
#[link(name = "polkit-agent-1")]
unsafe extern "C" {
    fn polkit_agent_session_new(
        identity: *mut GObject,
        cookie: *const libc::c_char,
    ) -> *mut GObject;
    fn polkit_agent_session_initiate(session: *mut GObject);
    fn polkit_agent_session_cancel(session: *mut GObject);
    fn polkit_agent_session_response(session: *mut GObject, response: *const libc::c_char);
}

#[derive(Clone)]
pub struct Prompt {
    pub id: u64,
    pub message: String,
    pub action: String,
    pub details: BTreeMap<String, String>,
    pub users: Vec<(u32, String, String)>,
    pub selected: u32,
    pub failures: usize,
    pub status: String,
    pub challenge: Option<(u64, String, bool)>,
}
struct Pending {
    prompt: Prompt,
    cookie: CString,
    reply: gio::DBusMethodInvocation,
    session: Option<glib::Object>,
}
pub struct Agent {
    bus: gio::DBusConnection,
    reader: Rc<Reader>,
    tx: Sender<Event>,
    owner: RefCell<String>,
    subject: glib::Variant,
    active: RefCell<Option<Pending>>,
    locked: Cell<bool>,
    serial: Cell<u64>,
}
impl Agent {
    pub fn new(reader: Rc<Reader>, tx: Sender<Event>) -> anyhow::Result<Rc<Self>> {
        let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)?;
        let sid = std::env::var("XDG_SESSION_ID")?;
        let subject = (
            "unix-session",
            BTreeMap::from([("session-id", sid.to_variant())]),
        )
            .to_variant();
        let a = Rc::new(Self {
            bus: bus.clone(),
            reader,
            tx,
            subject,
            owner: Default::default(),
            active: Default::default(),
            locked: Cell::new(true),
            serial: Cell::new(0),
        });
        let agent = a.clone();
        bus.register_object(AGENT, &gio::DBusNodeInfo::for_xml(XML)?.interfaces()[0])
            .method_call(move |_, sender, _, _, method, args, reply| {
                if sender.is_none() || sender != Some(agent.owner.borrow().as_str()) {
                    reply.return_dbus_error(
                        "org.freedesktop.DBus.Error.AccessDenied",
                        "Authority required",
                    );
                    return;
                }
                if method == "CancelAuthentication" {
                    let cookie = args.child_get::<String>(0);
                    if agent
                        .active
                        .borrow()
                        .as_ref()
                        .is_some_and(|p| p.cookie.as_bytes() == cookie.as_bytes())
                    {
                        agent.finish(true)
                    }
                    reply.return_value(None);
                } else {
                    agent.begin(args, reply)
                }
            })
            .build()?;
        let up = a.clone();
        let down = a.clone();
        gio::bus_watch_name_on_connection(
            &bus,
            AUTH,
            gio::BusNameWatcherFlags::NONE,
            move |_, _, owner| {
                let locale = glib::language_names()
                    .first()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "C".into());
                let args = glib::Variant::tuple_from_iter([
                    up.subject.clone(),
                    locale.to_variant(),
                    AGENT.to_variant(),
                ]);
                match up.call(owner, "RegisterAuthenticationAgent", &args) {
                    Ok(_) => {
                        up.owner.replace(owner.into());
                    }
                    Err(e) => eprintln!("Register polkit agent: {e}"),
                }
            },
            move |_, _| {
                down.owner.borrow_mut().clear();
                down.finish(true)
            },
        );
        Ok(a)
    }
    fn call(
        &self,
        owner: &str,
        method: &str,
        args: &glib::Variant,
    ) -> Result<glib::Variant, glib::Error> {
        self.bus.call_sync(
            Some(owner),
            PATH,
            AUTHORITY,
            method,
            Some(args),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
    }
    fn next(&self) -> u64 {
        let n = self.serial.get() + 1;
        self.serial.set(n);
        n
    }
    fn begin(self: &Rc<Self>, args: glib::Variant, reply: gio::DBusMethodInvocation) {
        if self.locked.get() || self.active.borrow().is_some() {
            reply.return_dbus_error(CANCELLED, "Authentication unavailable");
            return;
        }
        let ids: Vec<(String, BTreeMap<String, glib::Variant>)> = args.child_get(5);
        let users: Vec<_> = ids
            .into_iter()
            .filter_map(|(kind, attrs)| {
                if kind != "unix-user" {
                    return None;
                }
                let uid = attrs.get("uid")?.get::<u32>()?;
                if uid > i32::MAX as u32 {
                    return None;
                }
                let user = nix::unistd::User::from_uid(uid.into()).ok().flatten()?;
                let full = user
                    .gecos
                    .to_string_lossy()
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                Some((uid, user.name, full))
            })
            .collect();
        let Some(&(selected, _, _)) = users.first() else {
            reply.return_dbus_error(
                "org.freedesktop.PolicyKit1.Error.Failed",
                "No supported administrator",
            );
            return;
        };
        let id = self.next();
        self.reader.sensitive(true);
        self.active.replace(Some(Pending {
            prompt: Prompt {
                id,
                action: args.child_get(0),
                message: args.child_get(1),
                details: args.child_get(3),
                users,
                selected,
                failures: 0,
                status: String::new(),
                challenge: None,
            },
            cookie: CString::new(args.child_get::<String>(4)).unwrap(),
            reply,
            session: None,
        }));
        self.select(id, selected);
    }
    pub fn select(self: &Rc<Self>, id: u64, uid: u32) {
        let mut state = self.active.borrow_mut();
        let Some(p) = state
            .as_mut()
            .filter(|p| p.prompt.id == id && p.prompt.users.iter().any(|u| u.0 == uid))
        else {
            return;
        };
        let old = p.session.take();
        p.prompt.selected = uid;
        p.prompt.challenge = None;
        if old.is_some() {
            p.prompt.status.clear();
        }
        let user: glib::Object = unsafe { from_glib_full(polkit_unix_user_new(uid as i32)) };
        let session: glib::Object =
            unsafe { from_glib_full(polkit_agent_session_new(user.as_ptr(), p.cookie.as_ptr())) };
        p.session = Some(session.clone());
        let _ = self.tx.try_send(Event::Auth(p.prompt.clone()));
        drop(state);
        if let Some(old) = old {
            unsafe { polkit_agent_session_cancel(old.as_ptr()) }
        }
        for signal in ["request", "show-info", "show-error", "completed"] {
            let weak = Rc::downgrade(self);
            session.connect_local(signal, false, move |values| {
                let a = weak.upgrade()?;
                let emitting = values[0].get::<glib::Object>().ok()?;
                let mut state = a.active.borrow_mut();
                let p = state
                    .as_mut()
                    .filter(|p| p.session.as_ref() == Some(&emitting))?;
                if signal == "completed" {
                    p.session.take();
                    let success = values[1].get::<bool>().ok()?;
                    if success {
                        drop(state);
                        a.finish(false);
                    } else {
                        p.prompt.challenge = None;
                        p.prompt.failures += 1;
                        let last = p.prompt.failures >= 3;
                        p.prompt.status = if last {
                            "Authentication failed."
                        } else {
                            "Authentication failed. Try again."
                        }
                        .into();
                        let id = p.prompt.id;
                        let uid = p.prompt.selected;
                        let _ = a.tx.try_send(Event::Auth(p.prompt.clone()));
                        drop(state);
                        // A new helper performs every retry, with the original authority cookie.
                        // Defer to avoid recursing if starting the helper itself fails.
                        let a = a.clone();
                        glib::MainContext::ref_thread_default().spawn_local(async move {
                            let delay = if last { 1200 } else { 250 };
                            glib::timeout_future(std::time::Duration::from_millis(delay)).await;
                            if a.active
                                .borrow()
                                .as_ref()
                                .is_some_and(|p| p.prompt.id == id && p.session.is_none())
                            {
                                if last {
                                    a.finish(false);
                                } else {
                                    a.select(id, uid);
                                }
                            }
                        });
                    }
                } else {
                    let text = values[1].get::<String>().ok()?;
                    if signal == "request" {
                        p.prompt.challenge = Some((a.next(), text, values[2].get::<bool>().ok()?))
                    } else {
                        p.prompt.status = text
                    }
                    let _ = a.tx.try_send(Event::Auth(p.prompt.clone()));
                }
                None
            });
        }
        unsafe { polkit_agent_session_initiate(session.as_ptr()) }
    }
    pub fn respond(&self, id: u64, challenge: u64, answer: &str) {
        let mut state = self.active.borrow_mut();
        let Some(p) = state.as_mut().filter(|p| {
            p.prompt.id == id
                && p.prompt
                    .challenge
                    .as_ref()
                    .is_some_and(|c| c.0 == challenge)
        }) else {
            return;
        };
        if answer.contains(['\0', '\r', '\n']) {
            drop(state);
            self.finish(true);
            return;
        }
        let session = p.session.clone();
        p.prompt.challenge = None;
        let _ = self.tx.try_send(Event::Auth(p.prompt.clone()));
        drop(state);
        if let Some(session) = session {
            let mut bytes = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
            bytes.extend_from_slice(answer.as_bytes());
            bytes.push(0);
            unsafe { polkit_agent_session_response(session.as_ptr(), bytes.as_ptr().cast()) }
        }
    }
    pub fn cancel(&self, id: u64) {
        if self
            .active
            .borrow()
            .as_ref()
            .is_some_and(|p| p.prompt.id == id)
        {
            self.finish(true)
        }
    }
    pub fn lock(&self, value: bool) {
        self.locked.set(value);
        if value {
            self.finish(true)
        }
    }
    fn finish(&self, cancel: bool) {
        let Some(p) = self.active.take() else { return };
        if let Some(s) = p.session {
            unsafe { polkit_agent_session_cancel(s.as_ptr()) }
        }
        if cancel {
            p.reply
                .return_dbus_error(CANCELLED, "Authentication cancelled")
        } else {
            p.reply.return_value(None)
        }
        let _ = self.tx.try_send(Event::AuthClosed(p.prompt.id));
        self.reader.sensitive(false);
    }
    pub fn shutdown(&self) {
        self.lock(true);
        let owner = self.owner.take();
        if !owner.is_empty() {
            let args = glib::Variant::tuple_from_iter([self.subject.clone(), AGENT.to_variant()]);
            let _ = self.call(&owner, "UnregisterAuthenticationAgent", &args);
        }
    }
}
