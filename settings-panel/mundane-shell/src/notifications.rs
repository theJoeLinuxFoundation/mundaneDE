//! Session-bus notification ownership, history and banner policy.
use crate::Event;
use async_channel::Sender;
use gio::prelude::*;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const XML: &str = r#"<node><interface name="org.freedesktop.Notifications">
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
<method name="Notify"><arg type="s"/><arg type="u"/><arg type="s"/><arg type="s"/><arg type="s"/><arg type="as"/><arg type="a{sv}"/><arg type="i"/><arg type="u" direction="out"/></method>
<method name="CloseNotification"><arg type="u"/></method>
<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
</interface></node>"#;
#[derive(Clone, PartialEq)]
pub struct Note {
    pub id: u32,
    pub source: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
}
impl Note {
    pub fn card(&self) -> mundane_ui::Notice {
        mundane_ui::Notice {
            key: self.id.to_string().into(),
            title: self.title.clone().into(),
            body: self.body.clone().into(),
            source: (!self.source.is_empty()).then(|| self.source.clone().into()),
            actions: self
                .actions
                .iter()
                .map(|(k, v)| (k.clone().into(), v.clone().into()))
                .collect(),
        }
    }
}
struct History {
    seq: u32,
    notes: VecDeque<(String, Note)>,
}
pub struct Server {
    state: RefCell<History>,
    bus: gio::DBusConnection,
    prefs: gio::Settings,
    tx: Sender<Event>,
}
impl Server {
    pub fn new(tx: Sender<Event>) -> anyhow::Result<Rc<Self>> {
        let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let server = Rc::new(Self {
            state: RefCell::new(History {
                seq: 0,
                notes: VecDeque::new(),
            }),
            bus: bus.clone(),
            prefs: gio::Settings::new("org.gnome.desktop.notifications"),
            tx,
        });
        let s = server.clone();
        let info = gio::DBusNodeInfo::for_xml(XML)?
            .lookup_interface(NAME)
            .unwrap();
        bus.register_object(PATH, &info)
            .method_call(move |_, sender, _, _, method, args, reply| {
                let sender = sender.unwrap_or("");
                let result = match method {
                    "GetCapabilities" => (vec!["actions", "body"],).to_variant(),
                    "GetServerInformation" => ("mundaneDE", "joe Linux", "1", "1.2").to_variant(),
                    "CloseNotification" => {
                        let id = args.child_get(0);
                        if s.state
                            .borrow()
                            .notes
                            .iter()
                            .any(|(owner, n)| owner == sender && n.id == id)
                        {
                            s.close(id, 3);
                        }
                        ().to_variant()
                    }
                    "Notify" => {
                        let actions: Vec<String> = args.child_get(5);
                        let note = Note {
                            id: args.child_get(1),
                            source: args.child_get(0),
                            title: args.child_get(3),
                            body: args.child_get(4),
                            actions: actions
                                .chunks_exact(2)
                                .map(|a| (a[0].clone(), a[1].clone()))
                                .collect(),
                        };
                        (s.notify(sender, note, args.child_get(7)),).to_variant()
                    }
                    _ => {
                        reply.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method);
                        return;
                    }
                };
                reply.return_value(Some(&result));
            })
            .build()?;
        gio::bus_own_name_on_connection(
            &bus,
            NAME,
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| {},
        );
        let tx = server.tx.clone();
        server
            .prefs
            .connect_changed(Some("show-banners"), move |p, _| {
                if !p.boolean("show-banners") {
                    let _ = tx.try_send(Event::HideBanner(None));
                }
            });
        Ok(server)
    }
    fn snapshot(&self) {
        let _ = self.tx.try_send(Event::History(
            self.state
                .borrow()
                .notes
                .iter()
                .rev()
                .map(|(_, n)| n.clone())
                .collect(),
        ));
    }
    fn notify(&self, owner: &str, mut n: Note, timeout: i32) -> u32 {
        let mut state = self.state.borrow_mut();
        if let Some(pos) = state
            .notes
            .iter()
            .position(|(o, old)| o == owner && old.id == n.id)
        {
            state.notes.remove(pos);
        } else {
            loop {
                state.seq = state.seq.wrapping_add(1).max(1);
                if state.notes.iter().all(|(_, n)| n.id != state.seq) {
                    break;
                }
            }
            n.id = state.seq;
        }
        let id = n.id;
        state.notes.push_back((owner.to_owned(), n.clone()));
        let expired = (state.notes.len() > 64).then(|| state.notes[0].1.id);
        drop(state);
        if let Some(old) = expired {
            self.close(old, 1);
        } else {
            self.snapshot();
        }
        if self.prefs.boolean("show-banners") {
            let _ = self.tx.try_send(Event::Banner(n, timeout));
        }
        id
    }
    pub fn local(&self, title: String, body: String) {
        self.notify(
            "",
            Note {
                id: 0,
                source: "joe Linux".into(),
                title,
                body,
                actions: vec![],
            },
            -1,
        );
    }
    pub fn action(&self, id: u32, key: Option<String>) {
        if let Some(key) = key {
            let state = self.state.borrow();
            let Some((owner, n)) = state.notes.iter().find(|(_, n)| n.id == id) else {
                return;
            };
            if owner.is_empty() || !n.actions.iter().any(|(k, _)| k == &key) {
                return;
            }
            let _ = self.bus.emit_signal(
                Some(owner),
                PATH,
                NAME,
                "ActionInvoked",
                Some(&(id, key).to_variant()),
            );
        }
        self.close(id, 2);
    }
    fn close(&self, id: u32, reason: u32) {
        let mut state = self.state.borrow_mut();
        let Some(i) = state.notes.iter().position(|(_, n)| n.id == id) else {
            return;
        };
        let (owner, _) = state.notes.remove(i).unwrap();
        drop(state);
        if !owner.is_empty() {
            let _ = self.bus.emit_signal(
                Some(&owner),
                PATH,
                NAME,
                "NotificationClosed",
                Some(&(id, reason).to_variant()),
            );
        }
        let _ = self.tx.try_send(Event::HideBanner(Some(id)));
        self.snapshot();
    }
}
