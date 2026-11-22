"""Virtual BlueZ and UPower services, confined to the supplied private bus."""

import sys
import os
from pathlib import Path
from gi.repository import Gio, GLib

assert os.environ["DBUS_SYSTEM_BUS_ADDRESS"] == os.environ["DBUS_SESSION_BUS_ADDRESS"]
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
root = "/org/bluez/hci0/dev_11_22_33_44_55_66"
agent = None


def notify(text):
    with Path(sys.argv[1]).open("a") as f:
        print(text, file=f)


def method(_b, sender, path, iface, method, args, req):
    global agent
    if method == "RegisterAgent":
        agent = (sender, args.unpack()[0])
        req.return_value(None)
    if method == "SetBrightness":
        subsystem, name, raw = args.unpack()
        assert (subsystem, name) == ("backlight", "virt-panel") and 0 <= raw <= 400
        Path(sys.argv[2], "brightness").write_text(str(raw))
        notify(f"brightness {raw}")
        req.return_value(None)
    if method == "CancelPairing":
        req.return_value(None)
    if method == "GetManagedObjects":
        req.return_value(
            GLib.Variant(
                "(a{oa{sa{sv}}})",
                (
                    {
                        "/org/bluez/hci0": {
                            "org.bluez.Adapter1": {"Powered": GLib.Variant("b", True)}
                        },
                        root: {
                            "org.bluez.Device1": {
                                "Alias": GLib.Variant("s", "Guest keyboard"),
                                "Address": GLib.Variant("s", "11:22:33:44:55:66"),
                            }
                        },
                    },
                ),
            )
        )
    if method == "Pair":

        def confirmed(bus, result):
            try:
                bus.call_finish(result)
            except GLib.Error:
                notify("cancelled")
                req.return_dbus_error(
                    "org.bluez.Error.AuthenticationCanceled", "Cancelled"
                )
            else:
                notify("paired")
                req.return_value(None)

        bus.call(
            *agent,
            "org.bluez.Agent1",
            "RequestConfirmation",
            GLib.Variant("(ou)", (root, 123456)),
            None,
            Gio.DBusCallFlags.NONE,
            15000,
            None,
            confirmed,
        )


def register(path, xml, props=None):
    bus.register_object(
        path,
        Gio.DBusNodeInfo.new_for_xml("<node>" + xml + "</node>").interfaces[0],
        method,
        props,
        None,
    )


register(
    "/",
    '<interface name="org.freedesktop.DBus.ObjectManager"><method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method></interface>',
)
register(
    "/org/bluez",
    '<interface name="org.bluez.AgentManager1"><method name="RegisterAgent"><arg type="o"/><arg type="s"/></method></interface>',
)
register(
    root,
    '<interface name="org.bluez.Device1"><method name="Pair"/><method name="CancelPairing"/></interface>',
)
register(
    "/org/freedesktop/UPower/devices/DisplayDevice",
    '<interface name="org.freedesktop.UPower.Device"><property type="b" name="IsPresent" access="read"/><property type="d" name="Percentage" access="read"/></interface>',
    lambda b, s, p, i, key: (
        GLib.Variant("b", True) if key == "IsPresent" else GLib.Variant("d", 72.4)
    ),
)
names = ["org.bluez", "org.freedesktop.UPower"]
if len(sys.argv) == 3:
    register(
        "/org/freedesktop/login1/session/auto",
        '<interface name="org.freedesktop.login1.Session"><method name="SetBrightness"><arg type="s"/><arg type="s"/><arg type="u"/></method></interface>',
    )
    names.append("org.freedesktop.login1")
for name in names:
    Gio.bus_own_name_on_connection(bus, name, Gio.BusNameOwnerFlags.NONE, None, None)
GLib.MainLoop().run()
