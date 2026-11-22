"""BlueZ pairs on the same connection that supplies the confirmation dialog."""

from gi.repository import Gio, GLib

AGENT = "/org/joelinux/mundane/agent"
INTERFACE = "org.bluez.Agent1"
XML = """<node><interface name="org.bluez.Agent1">
<method name="Release"/><method name="Cancel"/>
<method name="RequestConfirmation"><arg type="o"/><arg type="u"/></method>
<method name="RequestAuthorization"><arg type="o"/></method>
<method name="AuthorizeService"><arg type="o"/><arg type="s"/></method>
<method name="RequestPasskey"><arg type="o"/><arg type="u" direction="out"/></method>
<method name="RequestPinCode"><arg type="o"/><arg type="s" direction="out"/></method>
<method name="DisplayPinCode"><arg type="o"/><arg type="s"/></method>
<method name="DisplayPasskey"><arg type="o"/><arg type="u"/><arg type="q"/></method>
</interface></node>"""


class Bluetooth:
    def __init__(self, panel, bus=None):
        self.panel = panel
        self.bus = bus or Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        self.registered = False
        self.request = None
        self.device = None
        self.owner = None
        self.bus.register_object(
            AGENT,
            Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0],
            self.method,
            None,
            None,
        )
        Gio.bus_watch_name_on_connection(
            self.bus,
            "org.bluez",
            Gio.BusNameWatcherFlags.NONE,
            self.appeared,
            self.vanished,
        )

    def appeared(self, bus, name, owner):
        self.owner = owner

    def vanished(self, *_):
        self.owner = None
        self.registered = False
        self.cancel()

    def cancel(self):
        if self.request:
            self.request.return_dbus_error(
                "org.bluez.Error.Rejected", "Pairing cancelled"
            )
            self.request = None
        if self.panel.page == "pairing":
            self.panel.dismiss()

    def method(self, bus, sender, path, iface, method, args, request):
        if sender != self.owner:
            request.return_dbus_error(
                "org.freedesktop.DBus.Error.AccessDenied", "BlueZ agent"
            )
            return
        if method in ("Release", "Cancel"):
            if method == "Release":
                self.registered = False
            self.cancel()
            request.return_value(None)
            return
        values = args.unpack()
        if self.device != values[0]:
            request.return_dbus_error(
                "org.bluez.Error.Rejected", "No pairing requested for device"
            )
            return
        if method.startswith("Display"):
            self.panel.bus.notify("Bluetooth passkey", str(values[1]))
            request.return_value(None)
            return
        self.cancel()
        self.request = request
        self.panel.pairing(method, values, self.answer)

    def answer(self, value=None):
        request, self.request = self.request, None
        if request:
            if value is False:
                request.return_dbus_error(
                    "org.bluez.Error.Rejected", "Pairing cancelled"
                )
            else:
                request.return_value(value)

    def pair(self, addr):
        try:
            if not self.owner:
                raise ValueError("Bluetooth service unavailable")
            tree = self.bus.call_sync(
                self.owner,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "GetManagedObjects",
                None,
                None,
                Gio.DBusCallFlags.NONE,
                1500,
                None,
            ).unpack()[0]
            self.device = next(
                (
                    path
                    for path, interfaces in tree.items()
                    if interfaces.get("org.bluez.Device1", {}).get("Address") == addr
                ),
                None,
            )
            if not self.device:
                raise ValueError("Device is no longer available")
            if not self.registered:
                self.bus.call_sync(
                    self.owner,
                    "/org/bluez",
                    "org.bluez.AgentManager1",
                    "RegisterAgent",
                    GLib.Variant("(os)", (AGENT, "KeyboardDisplay")),
                    None,
                    Gio.DBusCallFlags.NONE,
                    3000,
                    None,
                )
                self.registered = True
            self.bus.call(
                self.owner,
                self.device,
                "org.bluez.Device1",
                "Pair",
                None,
                None,
                Gio.DBusCallFlags.NONE,
                60000,
                None,
                self.finished,
            )
        except (GLib.Error, ValueError) as e:
            self.panel.bus.notify("Bluetooth pairing unavailable", str(e))

    def finished(self, bus, result):
        try:
            bus.call_finish(result)
            self.panel.bus.notify("Bluetooth device paired", "")
        except GLib.Error as e:
            self.panel.bus.notify("Bluetooth pairing ended", e.message)
        self.device = None
        self.cancel()
