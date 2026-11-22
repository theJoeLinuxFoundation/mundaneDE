"""freedesktop notifications and the StatusNotifierItem watcher on the user bus."""

from gi.repository import Gio, GLib

NOTES = "org.freedesktop.Notifications"
WATCH = "org.kde.StatusNotifierWatcher"
ITEM = "org.kde.StatusNotifierItem"
NOTE_PATH = "/org/freedesktop/Notifications"
TRAY_PATH = "/StatusNotifierWatcher"
NOTE_XML = """<node><interface name="org.freedesktop.Notifications">
<method name="GetCapabilities"><arg type="as" direction="out"/></method>
<method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
<method name="Notify"><arg type="s"/><arg type="u"/><arg type="s"/><arg type="s"/><arg type="s"/><arg type="as"/><arg type="a{sv}"/><arg type="i"/><arg type="u" direction="out"/></method>
<method name="CloseNotification"><arg type="u"/></method>
<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
</interface></node>"""
TRAY_XML = """<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/>
</interface></node>"""


class Services:
    def __init__(self, panel):
        self.panel = panel
        self.items = {}
        self.notes = {}
        self.sequence = 0
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        for name, path, xml in [
            (NOTES, NOTE_PATH, NOTE_XML),
            (WATCH, TRAY_PATH, TRAY_XML),
        ]:
            interface = Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0]
            self.bus.register_object(path, interface, self.method, self.prop, None)
            Gio.bus_own_name_on_connection(
                self.bus, name, Gio.BusNameOwnerFlags.NONE, None, None
            )
        self.bus.signal_subscribe(
            "org.freedesktop.DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            None,
            None,
            Gio.DBusSignalFlags.NONE,
            self.owner_changed,
        )
        self.bus.signal_subscribe(
            None, ITEM, None, None, None, Gio.DBusSignalFlags.NONE, self.item_changed
        )

    def prop(self, bus, sender, path, iface, key):
        return {
            "RegisteredStatusNotifierItems": GLib.Variant("as", list(self.items)),
            "ProtocolVersion": GLib.Variant("i", 0),
            "IsStatusNotifierHostRegistered": GLib.Variant("b", True),
        }[key]

    def owner_changed(self, bus, sender, path, iface, signal, args):
        name, old, new = args.unpack()
        if not old or old == new:
            return
        removed = False
        for ident, (dest, obj, owner) in list(self.items.items()):
            if name in (owner, dest):
                del self.items[ident]
                removed = True
                self.signal(
                    TRAY_PATH, WATCH, "StatusNotifierItemUnregistered", "(s)", (ident,)
                )
        if removed:
            self.panel.refresh_trays()

    def item_changed(self, *_):
        self.panel.refresh_trays()

    def signal(self, path, interface, name, signature, values, destination=None):
        self.bus.emit_signal(
            destination, path, interface, name, GLib.Variant(signature, values)
        )

    def method(self, bus, sender, path, interface, method, args, request):
        v = args.unpack()
        if method == "Notify":
            _, replace, _, title, body, actions, _, timeout = v
            n = self.notify(title, body, sender, actions, replace, timeout)
            request.return_value(GLib.Variant("(u)", (n,)))
        elif method == "CloseNotification":
            if v[0] in self.notes and self.notes[v[0]][2] == sender:
                self.close(v[0], 3)
            request.return_value(None)
        elif method == "GetCapabilities":
            request.return_value(GLib.Variant("(as)", (["actions", "body"],)))
        elif method == "GetServerInformation":
            request.return_value(
                GLib.Variant("(ssss)", ("mundaneDE", "joe Linux", "1", "1.2"))
            )
        elif method == "RegisterStatusNotifierItem":
            dest, obj = (
                (sender, v[0])
                if v[0].startswith("/")
                else (v[0], "/StatusNotifierItem")
            )
            ident = dest + obj
            self.items[ident] = (dest, obj, sender)
            self.signal(
                TRAY_PATH, WATCH, "StatusNotifierItemRegistered", "(s)", (ident,)
            )
            request.return_value(None)
            GLib.idle_add(self.panel.refresh_trays)
        elif method == "RegisterStatusNotifierHost":
            request.return_value(None)

    def notify(self, title, body, sender=None, actions=(), replace=0, timeout=-1):
        if replace not in self.notes or self.notes[replace][2] != sender:
            self.sequence += 1
            replace = self.sequence
        self.notes[replace] = (
            title,
            body,
            sender,
            dict(zip(actions[::2], actions[1::2])),
        )
        while len(self.notes) > 64:
            self.close(next(iter(self.notes)), 1)
        self.panel.toast(replace, title, body, timeout)
        return replace

    def action(self, n, key):
        note = self.notes.get(n)
        if note and note[2]:
            self.signal(NOTE_PATH, NOTES, "ActionInvoked", "(us)", (n, key), note[2])
        self.close(n, 2)

    def close(self, n, reason):
        note = self.notes.pop(n, None)
        if note and note[2]:
            self.signal(
                NOTE_PATH, NOTES, "NotificationClosed", "(uu)", (n, reason), note[2]
            )
        self.panel.hide_toast(n)
        if self.panel.page == "system":
            self.panel.draw_system()

    def tray_info(self, dest, obj):
        return self.bus.call_sync(
            dest,
            obj,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            GLib.Variant("(s)", (ITEM,)),
            None,
            Gio.DBusCallFlags.NONE,
            800,
            None,
        ).unpack()[0]

    def activate_tray(self, dest, obj):
        self.bus.call(
            dest,
            obj,
            ITEM,
            "Activate",
            GLib.Variant("(ii)", (0, 0)),
            None,
            Gio.DBusCallFlags.NONE,
            3000,
            None,
            None,
        )


class Shares:
    """M6's portal publishes active streams through the D-Bus ObjectManager."""

    backend = "org.freedesktop.impl.portal.desktop.mundane"
    root = "/org/freedesktop/portal/desktop"
    marker = "org.joelinux.mundane.ActiveShare"

    def __init__(self, bus, changed):
        self.bus = bus
        self.changed = changed
        self.owner = None
        self.active = {}
        self.subscription = 0
        Gio.bus_watch_name_on_connection(
            bus,
            self.backend,
            Gio.BusNameWatcherFlags.NONE,
            self.appeared,
            self.vanished,
        )

    def appeared(self, bus, name, owner):
        self.owner = owner
        self.subscription = bus.signal_subscribe(
            owner,
            "org.freedesktop.DBus.ObjectManager",
            None,
            self.root,
            None,
            Gio.DBusSignalFlags.NONE,
            lambda *_: self.refresh(),
        )
        self.refresh()

    def vanished(self, *_):
        self.owner = None
        if self.subscription:
            self.bus.signal_unsubscribe(self.subscription)
            self.subscription = 0
        self.active = {}
        self.changed()

    def refresh(self):
        owner = self.owner
        if not owner:
            return

        def got(bus, res):
            if self.owner != owner:
                return
            try:
                objects = bus.call_finish(res).unpack()[0]
            except GLib.Error:
                return
            self.active = {
                p: props[self.marker].get("Source", "Display")
                for p, props in objects.items()
                if self.marker in props
            }
            self.changed()

        self.bus.call(
            owner,
            self.root,
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None,
            None,
            Gio.DBusCallFlags.NONE,
            2000,
            None,
            got,
        )

    def stop(self, path):
        if path in self.active and self.owner:
            self.bus.call(
                self.owner,
                path,
                "org.freedesktop.impl.portal.Session",
                "Close",
                None,
                None,
                Gio.DBusCallFlags.NONE,
                2000,
                None,
                None,
            )
