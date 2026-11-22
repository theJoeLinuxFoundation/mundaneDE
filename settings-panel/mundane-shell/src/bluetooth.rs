//! A BlueZ agent for pairing initiated from this menu on this bus connection.
use crate::{devices::call, Event};
use async_channel::Sender;
use gio::{glib::variant::ObjectPath, prelude::*};
use std::{cell::RefCell, rc::Rc};
const AGENT: &str = "/org/joelinux/mundane/agent";
const XML: &str = r#"<node><interface name="org.bluez.Agent1">
<method name="Release"/><method name="Cancel"/>
<method name="RequestConfirmation"><arg type="o"/><arg type="u"/></method>
<method name="RequestAuthorization"><arg type="o"/></method>
<method name="AuthorizeService"><arg type="o"/><arg type="s"/></method>
<method name="RequestPasskey"><arg type="o"/><arg type="u" direction="out"/></method>
<method name="RequestPinCode"><arg type="o"/><arg type="s" direction="out"/></method>
<method name="DisplayPinCode"><arg type="o"/><arg type="s"/></method>
<method name="DisplayPasskey"><arg type="o"/><arg type="u"/><arg type="q"/></method>
</interface></node>"#;
#[derive(Clone, PartialEq)]
pub struct Prompt {
    pub serial: u64,
    pub text: String,
    pub method: String,
}
#[derive(Default)]
struct State {
    owner: String,
    device: String,
    name: String,
    registered: bool,
    generation: u64,
    serial: u64,
    pending: Option<(String, gio::DBusMethodInvocation)>,
}
pub struct Agent {
    bus: gio::DBusConnection,
    state: Rc<RefCell<State>>,
    tx: Sender<Event>,
}
impl Agent {
    pub fn new(tx: Sender<Event>) -> anyhow::Result<Rc<Self>> {
        let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)?;
        let agent = Rc::new(Self {
            bus: bus.clone(),
            state: Default::default(),
            tx,
        });
        let info = gio::DBusNodeInfo::for_xml(XML)?
            .lookup_interface("org.bluez.Agent1")
            .unwrap();
        let a = agent.clone();
        bus.register_object(AGENT, &info)
            .method_call(move |_, sender, _, _, method, args, reply| {
                let mut s = a.state.borrow_mut();
                if sender != Some(s.owner.as_str()) {
                    reply
                        .return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "BlueZ only");
                    return;
                }
                if matches!(method, "Release" | "Cancel") {
                    if method == "Release" {
                        s.registered = false;
                    }
                    drop(s);
                    a.cancel();
                    reply.return_value(None);
                    return;
                }
                if args.child_get::<ObjectPath>(0).as_str() != s.device {
                    reply.return_dbus_error("org.bluez.Error.Rejected", "No pairing requested");
                    return;
                }
                if let Some((_, old)) = s.pending.take() {
                    old.return_dbus_error("org.bluez.Error.Rejected", "Replaced");
                }
                s.serial += 1;
                let detail = match method {
                    "RequestConfirmation" => {
                        format!("Confirm passkey {:06}", args.child_get::<u32>(1))
                    }
                    "DisplayPasskey" => {
                        format!("Enter {:06} on the device", args.child_get::<u32>(1))
                    }
                    "DisplayPinCode" => {
                        format!("Enter {} on the device", args.child_get::<String>(1))
                    }
                    "RequestPasskey" | "RequestPinCode" => "Enter the device passkey".into(),
                    _ => "Authorize pairing".into(),
                };
                let _ = a.tx.try_send(Event::PairPrompt(Some(Prompt {
                    serial: s.serial,
                    text: format!("{} · {detail}", s.name),
                    method: method.into(),
                })));
                if method.starts_with("Display") {
                    reply.return_value(None);
                } else {
                    s.pending = Some((method.into(), reply));
                }
            })
            .build()?;
        let a = agent.clone();
        let b = agent.clone();
        gio::bus_watch_name_on_connection(
            &bus,
            "org.bluez",
            gio::BusNameWatcherFlags::NONE,
            move |_, _, owner| a.state.borrow_mut().owner = owner.into(),
            move |_, _| {
                b.cancel();
                let mut s = b.state.borrow_mut();
                s.owner.clear();
                s.registered = false;
            },
        );
        Ok(agent)
    }
    pub fn pair(self: &Rc<Self>, path: String, name: String) -> anyhow::Result<()> {
        self.cancel();
        let mut s = self.state.borrow_mut();
        anyhow::ensure!(!s.owner.is_empty(), "Bluetooth service unavailable");
        if !s.registered {
            call(
                &self.bus,
                &s.owner,
                "/org/bluez",
                "org.bluez.AgentManager1",
                "RegisterAgent",
                Some(&(ObjectPath::try_from(AGENT)?, "KeyboardDisplay").to_variant()),
            )?;
            s.registered = true;
        }
        s.device = path.clone();
        s.name = name;
        s.generation += 1;
        let generation = s.generation;
        let a = self.clone();
        self.bus.call(
            Some(&s.owner),
            &path,
            "org.bluez.Device1",
            "Pair",
            None,
            None,
            gio::DBusCallFlags::NONE,
            60000,
            gio::Cancellable::NONE,
            move |result| {
                if a.state.borrow().generation != generation {
                    return;
                }
                a.clear();
                let _ = a.tx.try_send(Event::DeviceMessage(
                    result
                        .map(|_| "Bluetooth device paired".into())
                        .unwrap_or_else(|e| e.message().into()),
                ));
            },
        );
        Ok(())
    }
    fn clear(&self) {
        let mut s = self.state.borrow_mut();
        s.generation += 1;
        s.device.clear();
        if let Some((_, reply)) = s.pending.take() {
            reply.return_dbus_error("org.bluez.Error.Rejected", "Pairing cancelled");
        }
        let _ = self.tx.try_send(Event::PairPrompt(None));
    }
    pub fn cancel(&self) {
        let s = self.state.borrow();
        if s.device.is_empty() {
            return;
        }
        self.bus.call(
            Some(&s.owner),
            &s.device,
            "org.bluez.Device1",
            "CancelPairing",
            None,
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
            |_| {},
        );
        drop(s);
        self.clear();
    }
    pub fn answer(&self, id: u64, input: String) {
        let mut s = self.state.borrow_mut();
        if id != s.serial {
            return;
        }
        let Some((method, reply)) = s.pending.take() else {
            return;
        };
        let args = match method.as_str() {
            "RequestPasskey" => match input.parse::<u32>() {
                Ok(v) if input.len() <= 6 && v < 1000000 => Some((v,).to_variant()),
                _ => {
                    s.pending = Some((method, reply));
                    return;
                }
            },
            "RequestPinCode" if input.is_empty() || input.len() > 16 => {
                s.pending = Some((method, reply));
                return;
            }
            "RequestPinCode" => Some((input,).to_variant()),
            _ => None,
        };
        reply.return_value(args.as_ref());
        let _ = self.tx.try_send(Event::PairPrompt(None));
    }
}
