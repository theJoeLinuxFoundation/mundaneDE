//! org.freedesktop.impl.portal.Settings on the session bus used by the portal frontend.
use gio::{glib::Variant, prelude::*};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

const ROOT: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.impl.portal.Settings";
const DESKTOP: &str = "org.gnome.desktop.interface";
const APPEARANCE: &str = "org.freedesktop.appearance";
const XML: &str = r#"<node><interface name="org.freedesktop.impl.portal.Settings">
<method name="ReadAll"><arg type="as" direction="in"/><arg type="a{sa{sv}}" direction="out"/></method>
<method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
<signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal>
<property name="version" type="u" access="read"/></interface></node>"#;
type Groups = BTreeMap<String, BTreeMap<String, Variant>>;

fn values(desktop: &gio::Settings, a11y: &gio::Settings) -> Groups {
    let accent = match desktop.string("accent-color").as_str() {
        "teal" => 0x2190a4,
        "green" => 0x3a944a,
        "yellow" => 0xc88800,
        "orange" => 0xed5b00,
        "red" => 0xe62d42,
        "pink" => 0xd56199,
        "purple" => 0x9141ac,
        "slate" => 0x6f8396,
        _ => 0x3584e4,
    };
    let channel = |shift: u32| f64::from((accent >> shift) & 255) / 255.;
    let scheme = match desktop.string("color-scheme").as_str() {
        "prefer-dark" => 1u32,
        "prefer-light" => 2,
        _ => 0,
    };
    BTreeMap::from([
        (
            APPEARANCE.into(),
            BTreeMap::from([
                ("color-scheme".into(), scheme.to_variant()),
                (
                    "accent-color".into(),
                    (channel(16), channel(8), channel(0)).to_variant(),
                ),
                (
                    "contrast".into(),
                    u32::from(a11y.boolean("high-contrast")).to_variant(),
                ),
                (
                    "reduced-motion".into(),
                    u32::from(!desktop.boolean("enable-animations")).to_variant(),
                ),
            ]),
        ),
        (
            DESKTOP.into(),
            [
                "font-name",
                "text-scaling-factor",
                "enable-animations",
                "color-scheme",
                "accent-color",
            ]
            .into_iter()
            .map(|k| (k.to_owned(), desktop.value(k)))
            .collect(),
        ),
    ])
}
pub fn register(acquired: impl Fn() + 'static) -> anyhow::Result<(gio::Settings, gio::Settings)> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    let desktop = gio::Settings::new(DESKTOP);
    let a11y = gio::Settings::new("org.gnome.desktop.a11y.interface");
    let state = Rc::new(RefCell::new(values(&desktop, &a11y)));
    let snapshot = state.clone();
    let info = gio::DBusNodeInfo::for_xml(XML)?
        .lookup_interface(IFACE)
        .unwrap();
    bus.register_object(ROOT, &info)
        .method_call(move |_, _, _, _, method, args, reply| {
            let groups = snapshot.borrow();
            if method == "ReadAll" {
                let (patterns,) = args.get::<(Vec<String>,)>().unwrap();
                let selected: Groups = groups
                    .iter()
                    .filter(|(name, _)| {
                        patterns.is_empty()
                            || patterns.iter().any(|p| {
                                name.as_str() == p
                                    || p.strip_suffix('*')
                                        .is_some_and(|prefix| name.starts_with(prefix))
                            })
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                reply.return_value(Some(&(selected,).to_variant()));
            } else {
                let (group, key) = args.get::<(String, String)>().unwrap();
                if let Some(value) = groups.get(&group).and_then(|m| m.get(&key)) {
                    reply.return_value(Some(&(value,).to_variant()));
                } else {
                    reply.return_dbus_error(
                        "org.freedesktop.portal.Error.NotFound",
                        "Unknown setting",
                    );
                }
            }
        })
        .property(|_, _, _, _, _| 1u32.to_variant())
        .build()?;
    for settings in [&desktop, &a11y] {
        let (desktop, a11y, bus, state) =
            (desktop.clone(), a11y.clone(), bus.clone(), state.clone());
        settings.connect_changed(None, move |_, _| {
            let next = values(&desktop, &a11y);
            let old = state.replace(next.clone());
            for (group, keys) in next {
                for (key, value) in keys {
                    if old.get(&group).and_then(|m| m.get(&key)) != Some(&value) {
                        let _ = bus.emit_signal(
                            None,
                            ROOT,
                            IFACE,
                            "SettingChanged",
                            Some(&(&group, key, value).to_variant()),
                        );
                    }
                }
            }
        });
    }
    gio::bus_own_name_on_connection(
        &bus,
        "org.freedesktop.impl.portal.desktop.mundane",
        gio::BusNameOwnerFlags::NONE,
        move |_, _| acquired(),
        |_, _| eprintln!("Desktop portal bus name unavailable"),
    );
    Ok((desktop, a11y))
}
