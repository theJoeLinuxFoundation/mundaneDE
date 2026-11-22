"""Authorize the session's screen reader on its unique D-Bus connection."""

import json
import os
import subprocess
import uuid

from gi.repository import Gio, GLib

NAME = "org.freedesktop.a11y.Manager"
PATH = "/org/freedesktop/a11y/Manager"
IFACE = "org.freedesktop.a11y.KeyboardMonitor"
XML = f"""<node><interface name="{IFACE}">
<method name="WatchKeyboard"/><method name="UnwatchKeyboard"/>
<method name="GrabKeyboard"/><method name="UngrabKeyboard"/>
<method name="SetKeyGrabs"><arg type="au"/><arg type="a(uu)"/></method>
<signal name="KeyEvent"><arg type="b"/><arg type="u"/><arg type="u"/><arg type="u"/><arg type="q"/></signal>
</interface></node>"""


class Reader:
    def __init__(self, panel):
        self.panel = panel
        self.bus = panel.bus.bus
        self.child = None
        self.owner = None
        self.subscription = ""
        self.sensitive = False
        self.stopping = False
        self.all = False
        self.keys = []
        self.modifiers = []
        self.watching = False
        self.prefs = Gio.Settings.new("org.gnome.desktop.a11y.applications")
        self.bus.register_object(
            PATH, Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], self.method
        )
        self.bus.signal_subscribe(
            "org.freedesktop.DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            "/org/freedesktop/DBus",
            None,
            0,
            self.owner_changed,
        )
        Gio.bus_own_name_on_connection(self.bus, NAME, 0, self.acquired, None)

    def acquired(self, *_):
        self.prefs.connect("changed::screen-reader-enabled", self.preference)
        self.preference()

    def toggle(self):
        self.prefs.set_boolean(
            "screen-reader-enabled", not self.prefs.get_boolean("screen-reader-enabled")
        )

    def preference(self, *_):
        if self.prefs.get_boolean("screen-reader-enabled"):
            if self.child is None:
                try:
                    child = subprocess.Popen(
                        ["/usr/bin/orca", "--replace"], stdin=subprocess.DEVNULL
                    )
                except OSError as e:
                    self.panel.bus.notify("Screen reader unavailable", str(e))
                    return
                self.child = child

                def exited(pid, status):
                    child.returncode = os.waitstatus_to_exitcode(status)
                    if self.child is child:
                        self.child = None
                        self.clear()
                        if (
                            self.prefs.get_boolean("screen-reader-enabled")
                            and self.stopping
                        ):
                            self.preference()
                        self.stopping = False

                GLib.child_watch_add(GLib.PRIORITY_DEFAULT, child.pid, exited)
        elif self.child is not None:
            self.clear()
            self.stopping = True
            self.child.terminate()

    def clear(self):
        self.owner = None
        self.watching = self.all = False
        self.keys = []
        self.modifiers = []
        self.configure()

    def owner_changed(self, _b, _s, _p, _i, _m, args):
        name, _, new = args.unpack()
        if name == self.owner and not new:
            self.clear()

    def configure(self):
        enabled = (
            self.owner
            and (self.watching or self.all or self.keys or self.modifiers)
            and not self.sensitive
        )
        if not enabled:
            self.subscription = ""
        elif not self.subscription:
            self.subscription = uuid.uuid4().hex
        config = (
            dict(
                subscription=self.subscription,
                all=self.all,
                keys=self.keys,
                modifiers=self.modifiers,
            )
            if enabled
            else {}
        )
        self.panel.send("keyboard-monitor " + json.dumps(config, separators=(",", ":")))

    def method(self, bus, sender, path, iface, method, args, reply):
        if sender != self.owner:
            pid = bus.call_sync(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                GLib.Variant("(s)", (sender,)),
                None,
                0,
                1000,
                None,
            ).unpack()[0]
            if (
                self.owner
                or not self.child
                or self.child.pid != pid
                or not self.prefs.get_boolean("screen-reader-enabled")
            ):
                reply.return_dbus_error(
                    "org.freedesktop.DBus.Error.AccessDenied",
                    "The session screen reader is required",
                )
                return
            self.owner = sender
        if method == "SetKeyGrabs":
            self.modifiers, self.keys = args.unpack()
        elif method in ("GrabKeyboard", "UngrabKeyboard"):
            self.all = method == "GrabKeyboard"
        else:
            self.watching = method == "WatchKeyboard"
        self.configure()
        reply.return_value(None)

    def event(self, data):
        def forwarded(*_):
            self.panel.send("keyboard-forward " + str(data["serial"]))

        if (
            self.owner
            and self.subscription
            and data["subscription"] == self.subscription
            and not self.sensitive
        ):
            self.bus.emit_signal(
                self.owner,
                PATH,
                IFACE,
                "KeyEvent",
                GLib.Variant("(buuuq)", tuple(data["key"])),
            )
            # Order the directed signal before the application's Wayland key. A
            # round trip on the same connection lets libatspi receive the key
            # before the resulting edit is published on the accessibility bus.
            self.bus.call(
                self.owner,
                PATH,
                "org.freedesktop.DBus.Peer",
                "Ping",
                None,
                None,
                0,
                40,
                None,
                forwarded,
            )
        else:
            forwarded()
