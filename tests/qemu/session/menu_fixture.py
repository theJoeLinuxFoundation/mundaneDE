"""A menu-bearing StatusNotifierItem in the isolated guest."""

from pathlib import Path
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def prop(b, s, p, i, k):
    return {
        "ItemIsMenu": GLib.Variant("b", True),
        "Menu": GLib.Variant("o", "/Menu"),
        "Title": GLib.Variant("s", "Review tray"),
        "IconName": GLib.Variant("s", "folder-symbolic"),
        "Status": GLib.Variant("s", "Active"),
    }[k]


def call(b, s, p, i, method, args, response):
    if method == "GetLayout":

        def item(number, label, **properties):
            return GLib.Variant(
                "(ia{sv}av)",
                (number, {"label": GLib.Variant("s", label), **properties}, []),
            )

        parent = args.unpack()[0]
        children = (
            [item(3, "Week.txt")]
            if parent == 2
            else [
                item(1, "Open folder"),
                item(
                    2,
                    "Recent documents",
                    **{"children-display": GLib.Variant("s", "submenu")},
                ),
                item(4, "Keep offline", enabled=GLib.Variant("b", False)),
            ]
        )
        response.return_value(
            GLib.Variant("(u(ia{sv}av))", (1, (parent, {}, children)))
        )
    elif method == "Event":
        if args.unpack()[:2] == (1, "clicked"):
            Path(GLib.get_user_runtime_dir(), "tray-menu-action").touch()
        response.return_value(None)
    else:
        response.return_value(GLib.Variant("(b)", (False,)))


for path, xml in [
    (
        "/Item",
        '<interface name="org.kde.StatusNotifierItem"><property name="ItemIsMenu" type="b" access="read"/><property name="Menu" type="o" access="read"/><property name="Title" type="s" access="read"/><property name="IconName" type="s" access="read"/><property name="Status" type="s" access="read"/></interface>',
    ),
    (
        "/Menu",
        '<interface name="com.canonical.dbusmenu"><method name="AboutToShow"><arg type="i"/><arg direction="out" type="b"/></method><method name="GetLayout"><arg type="i"/><arg type="i"/><arg type="as"/><arg direction="out" type="u"/><arg direction="out" type="(ia{sv}av)"/></method><method name="Event"><arg type="i"/><arg type="s"/><arg type="v"/><arg type="u"/></method></interface>',
    ),
]:
    bus.register_object(
        path,
        Gio.DBusNodeInfo.new_for_xml("<node>" + xml + "</node>").interfaces[0],
        call,
        prop,
    )
bus.call_sync(
    "org.kde.StatusNotifierWatcher",
    "/StatusNotifierWatcher",
    "org.kde.StatusNotifierWatcher",
    "RegisterStatusNotifierItem",
    GLib.Variant("(s)", ("/Item",)),
    None,
    0,
    1500,
    None,
)
GLib.MainLoop().run()
