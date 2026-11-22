"""Exercise notification identity and replacement as ordinary guest applications."""

import sys

from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
name = "org.freedesktop.Notifications"
path = "/org/freedesktop/Notifications"


def notify(title, body, replace=0, actions=(), timeout=0):
    return bus.call_sync(
        name,
        path,
        name,
        "Notify",
        GLib.Variant(
            "(susssasa{sv}i)", ("Files", replace, "", title, body, actions, {}, timeout)
        ),
        None,
        0,
        5000,
        None,
    ).unpack()[0]


def closed_or_invoked(bus, sender, path, iface, method, args):
    print(method, *args.unpack(), flush=True)


mode = sys.argv[1]
if mode == "copy":
    bus.signal_subscribe(None, name, None, path, None, 0, closed_or_invoked)
    ident = notify("Copying files", "Trip photos")
    assert (
        notify(
            "Copy complete",
            "24 photos copied to Pictures.",
            ident,
            ["open", "Open Pictures"],
        )
        == ident
    )
    print("ID", ident, flush=True)
    GLib.MainLoop().run()
elif mode == "foreign":
    ident = int(sys.argv[2])
    bus.call_sync(
        name,
        path,
        name,
        "CloseNotification",
        GLib.Variant("(u)", (ident,)),
        None,
        0,
        5000,
        None,
    )
    separate = notify("Calendar", "Design review at 10:00", ident)
    assert separate != ident
elif mode == "close":
    loop = GLib.MainLoop()
    ident = notify("Closing transfer", "Already finished")

    def acknowledged(_bus, _sender, _path, _iface, _method, args):
        if args.unpack() == (ident, 3):
            print("Owner closed its notification", flush=True)
            loop.quit()

    bus.signal_subscribe(name, name, "NotificationClosed", path, None, 0, acknowledged)
    bus.call_sync(
        name,
        path,
        name,
        "CloseNotification",
        GLib.Variant("(u)", (ident,)),
        None,
        0,
        5000,
        None,
    )
    GLib.timeout_add_seconds(3, loop.quit)
    loop.run()
elif mode == "fill":
    for i in range(67):
        notify(f"Transfer {i + 1}", f"Completed transfer {i + 1}")
elif mode == "short":
    notify("Transfer complete", "Available in notification history.", timeout=1200)
