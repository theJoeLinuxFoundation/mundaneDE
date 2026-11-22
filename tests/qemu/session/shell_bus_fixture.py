"""A tray app and an actionable notification on the isolated guest's user bus."""

from pathlib import Path
from gi.repository import Gio, GLib

runtime = Path(GLib.get_user_runtime_dir())
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def record(message):
    with (runtime / "shell-bus-events").open("a") as f:
        print(message, file=f)


def call(_bus, sender, path, iface, method, args, response):
    if method == "Activate":
        record("tray activated")
        response.return_value(None)


def export(path, xml, props=None):
    info = Gio.DBusNodeInfo.new_for_xml("<node>" + xml + "</node>").interfaces[0]
    bus.register_object(path, info, call, props, None)


def prop(_b, _s, _p, _i, key):
    return GLib.Variant(
        "s",
        {"Title": "Tray test", "IconName": "utilities-terminal", "Status": "Active"}[
            key
        ],
    )


export(
    "/StatusNotifierItem",
    """<interface name="org.kde.StatusNotifierItem">
<property name="Title" type="s" access="read"/><property name="IconName" type="s" access="read"/>
<property name="Status" type="s" access="read"/><method name="Activate"><arg type="i"/><arg type="i"/></method></interface>""",
    prop,
)


def registered(_bus, _name, _owner):
    bus.call(
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
        "RegisterStatusNotifierItem",
        GLib.Variant("(s)", ("/StatusNotifierItem",)),
        None,
        Gio.DBusCallFlags.NONE,
        2000,
        None,
        None,
    )


def notification(*_):
    record("notification action")


Gio.bus_watch_name_on_connection(
    bus, "org.kde.StatusNotifierWatcher", Gio.BusNameWatcherFlags.NONE, registered, None
)
bus.signal_subscribe(
    None,
    "org.freedesktop.Notifications",
    "ActionInvoked",
    None,
    None,
    Gio.DBusSignalFlags.NONE,
    notification,
)
bus.call_sync(
    "org.freedesktop.Notifications",
    "/org/freedesktop/Notifications",
    "org.freedesktop.Notifications",
    "Notify",
    GLib.Variant(
        "(susssasa{sv}i)",
        ("Fixture", 0, "", "Review file", "Saved", ["review", "Open review"], {}, 0),
    ),
    None,
    Gio.DBusCallFlags.NONE,
    2000,
    None,
)
GLib.MainLoop().run()
