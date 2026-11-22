//! StatusNotifierWatcher and DBusMenu, confined to the service thread.
use crate::Event;
use async_channel::Sender;
use gio::{
    glib::{self, Variant, VariantDict},
    prelude::*,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const XML: &str = r#"<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/></interface></node>"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub key: String,
    owner: String,
    path: String,
    pub title: String,
    pub icon: Option<std::path::PathBuf>,
    pub menu: String,
    pub is_menu: bool,
    passive: bool,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: i32,
    pub label: String,
    pub submenu: bool,
    pub enabled: bool,
    pub checked: bool,
}

pub struct Tray {
    bus: gio::DBusConnection,
    items: RefCell<BTreeMap<String, Item>>,
    events: Sender<Event>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
}
impl Tray {
    pub fn new(events: Sender<Event>) -> anyhow::Result<Rc<Self>> {
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)?;
        let tray = Rc::new(Self {
            bus: bus.clone(),
            items: Default::default(),
            events,
            subscriptions: Default::default(),
        });
        let info = gio::DBusNodeInfo::for_xml(XML)?
            .lookup_interface(WATCHER)
            .unwrap();
        let service = tray.clone();
        let props = tray.clone();
        bus.register_object(PATH, &info)
            .method_call(move |_, sender, _, _, method, args, reply| {
                if method == "RegisterStatusNotifierHost" {
                    service.signal("StatusNotifierHostRegistered", None);
                    reply.return_value(None);
                    return;
                }
                let address = args.child_get::<String>(0);
                let owner = sender.unwrap_or("");
                let (dest, path) = if address.starts_with('/') {
                    (owner, address.as_str())
                } else {
                    (address.as_str(), "/StatusNotifierItem")
                };
                if !glib::Variant::is_object_path(path)
                    || !gio::dbus_is_name(dest)
                    || (dest != owner
                        && service
                            .call(
                                "org.freedesktop.DBus",
                                "/org/freedesktop/DBus",
                                "org.freedesktop.DBus",
                                "GetNameOwner",
                                Some(&(dest,).to_variant()),
                            )
                            .ok()
                            .and_then(|r| r.get::<(String,)>())
                            .is_none_or(|r| r.0 != owner))
                {
                    reply.return_dbus_error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        "Item address must belong to the caller",
                    );
                    return;
                }
                let key = format!("{dest}{path}");
                let item = Item {
                    key: key.clone(),
                    owner: owner.into(),
                    path: path.into(),
                    title: dest.into(),
                    icon: None,
                    menu: String::new(),
                    is_menu: false,
                    passive: true,
                };
                service.items.borrow_mut().insert(key.clone(), item);
                reply.return_value(None);
                service.signal(
                    "StatusNotifierItemRegistered",
                    Some(&(key.as_str(),).to_variant()),
                );
                let service = service.clone();
                glib::MainContext::ref_thread_default().spawn_local(async move {
                    service.refresh(&key);
                });
            })
            .property(move |_, _, _, _, name| match name {
                "RegisteredStatusNotifierItems" => {
                    props.items.borrow().keys().collect::<Vec<_>>().to_variant()
                }
                "ProtocolVersion" => 0i32.to_variant(),
                _ => true.to_variant(),
            })
            .build()?;
        for (interface, member, subject) in [
            (ITEM, None, None),
            (
                "org.freedesktop.DBus.Properties",
                Some("PropertiesChanged"),
                Some(ITEM),
            ),
            (MENU, Some("LayoutUpdated"), None),
            (MENU, Some("ItemsPropertiesUpdated"), None),
        ] {
            let service = tray.clone();
            let sub = bus.subscribe_to_signal(
                None,
                Some(interface),
                member,
                None,
                subject,
                gio::DBusSignalFlags::NONE,
                move |s| {
                    let keys: Vec<_> = service
                        .items
                        .borrow()
                        .values()
                        .filter(|i| {
                            i.owner == s.sender_name
                                && (if interface == MENU { &i.menu } else { &i.path })
                                    == s.object_path
                        })
                        .map(|i| i.key.clone())
                        .collect();
                    for key in keys {
                        if interface == MENU {
                            let _ = service.events.try_send(Event::TrayChanged(key));
                        } else {
                            service.refresh(&key);
                        }
                    }
                },
            );
            tray.subscriptions.borrow_mut().push(sub);
        }
        let service = tray.clone();
        let owners = bus.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |s| {
                let Some((name, old, new)) = s.parameters.get::<(String, String, String)>() else {
                    return;
                };
                if old.is_empty() || new == old {
                    return;
                }
                let removed: Vec<_> = service
                    .items
                    .borrow()
                    .values()
                    .filter(|i| {
                        i.owner == old && (name == old || i.key == format!("{name}{}", i.path))
                    })
                    .map(|i| i.key.clone())
                    .collect();
                for key in &removed {
                    service.items.borrow_mut().remove(key);
                    service.signal("StatusNotifierItemUnregistered", Some(&(key,).to_variant()));
                }
                if !removed.is_empty() {
                    service.publish()
                }
            },
        );
        tray.subscriptions.borrow_mut().push(owners);
        gio::bus_own_name_on_connection(
            &bus,
            WATCHER,
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| eprintln!("Tray watcher name unavailable"),
        );
        Ok(tray)
    }
    fn signal(&self, name: &str, args: Option<&Variant>) {
        let _ = self.bus.emit_signal(None, PATH, WATCHER, name, args);
    }
    fn call(
        &self,
        dest: &str,
        path: &str,
        iface: &str,
        method: &str,
        args: Option<&Variant>,
    ) -> Result<Variant, glib::Error> {
        self.bus.call_sync(
            Some(dest),
            path,
            iface,
            method,
            args,
            None,
            gio::DBusCallFlags::NONE,
            1500,
            None::<&gio::Cancellable>,
        )
    }
    fn publish(&self) {
        let _ = self.events.try_send(Event::Tray(
            self.items
                .borrow()
                .values()
                .filter(|i| !i.passive)
                .cloned()
                .collect(),
        ));
    }
    fn refresh(&self, key: &str) {
        let mut items = self.items.borrow_mut();
        let Some(i) = items.get_mut(key) else { return };
        if let Ok(r) = self.call(
            &i.owner,
            &i.path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(ITEM,).to_variant()),
        ) {
            let d = VariantDict::new(Some(&r.child_value(0)));
            let string = |k| d.lookup::<String>(k).ok().flatten().unwrap_or_default();
            i.title = string("Title");
            if i.title.is_empty() {
                i.title = string("Id");
            }
            if i.title.is_empty() {
                i.title = i.key.clone();
            }
            i.passive = string("Status") == "Passive";
            i.is_menu = d.lookup("ItemIsMenu").ok().flatten().unwrap_or(false);
            i.menu = d
                .lookup_value("Menu", None)
                .and_then(|v| v.str().map(String::from))
                .filter(|p| p != "/NO_DBUSMENU")
                .unwrap_or_default();
            let theme = gio::Settings::new("org.gnome.desktop.interface").string("icon-theme");
            i.icon = crate::catalog::icon_path(&string("IconName"), &theme);
        }
        drop(items);
        self.publish();
    }
    pub fn activate(&self, key: &str) {
        if let Some(i) = self.items.borrow().get(key) {
            let _ = self.call(
                &i.owner,
                &i.path,
                ITEM,
                "Activate",
                Some(&(0i32, 0i32).to_variant()),
            );
        }
    }
    pub fn menu(&self, key: &str, parent: i32) -> anyhow::Result<Vec<Entry>> {
        let items = self.items.borrow();
        let i = items
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("Tray item closed"))?;
        let _ = self.call(
            &i.owner,
            &i.menu,
            MENU,
            "AboutToShow",
            Some(&(parent,).to_variant()),
        );
        let reply = self.call(
            &i.owner,
            &i.menu,
            MENU,
            "GetLayout",
            Some(&(parent, 1i32, Vec::<String>::new()).to_variant()),
        )?;
        anyhow::ensure!(
            reply.type_().as_str() == "(u(ia{sv}av))",
            "Invalid menu layout"
        );
        let mut rows = Vec::new();
        for child in reply.child_value(1).child_value(2).iter() {
            let Some(row) = child.as_variant() else {
                continue;
            };
            if row.type_().as_str() != "(ia{sv}av)" {
                continue;
            }
            let d = VariantDict::new(Some(&row.child_value(1)));
            if d.lookup::<bool>("visible")? == Some(false)
                || d.lookup::<String>("type")?.as_deref() == Some("separator")
            {
                continue;
            }
            let name = d
                .lookup::<String>("label")?
                .unwrap_or_default()
                .replace("__", "\0")
                .replace('_', "")
                .replace('\0', "_");
            rows.push(Entry {
                id: row.child_get(0),
                label: name,
                enabled: d.lookup("enabled")?.unwrap_or(true),
                checked: d.lookup("toggle-state")? == Some(1i32),
                submenu: d.lookup::<String>("children-display")?.as_deref() == Some("submenu"),
            });
        }
        Ok(rows)
    }
    pub fn select(&self, key: &str, id: i32) {
        if let Some(i) = self.items.borrow().get(key) {
            let _ = self.call(
                &i.owner,
                &i.menu,
                MENU,
                "Event",
                Some(&(id, "clicked", 0i32.to_variant(), 0u32).to_variant()),
            );
        }
    }
}
