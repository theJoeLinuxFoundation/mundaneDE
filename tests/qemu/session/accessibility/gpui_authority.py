"""Guest-only private authority driving the installed GPUI agent over D-Bus."""

import os
import sys
from pathlib import Path

from gi.repository import Gio, GLib

directory = Path(sys.argv[1])
assert os.environ["DBUS_SESSION_BUS_ADDRESS"] == f"unix:path={directory}/bus"
name = "org.freedesktop.PolicyKit1"
agent = name + ".AuthenticationAgent"
authority = name + ".Authority"
xml = f"""<node><interface name="{authority}">
<method name="RegisterAuthenticationAgent"><arg type="(sa{{sv}})"/><arg type="s"/><arg type="s"/></method>
<method name="UnregisterAuthenticationAgent"><arg type="(sa{{sv}})"/><arg type="s"/></method>
</interface></node>"""
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
target = None


def record(line):
    with (directory / "audit").open("a") as f:
        print(line, file=f)


def registration(connection, sender, path, iface, method, args, reply):
    global target
    subject, *rest = args.unpack()
    if subject != ("unix-session", {"session-id": sys.argv[2]}):
        reply.return_dbus_error(name + ".Error.Failed", "Wrong GDM session")
        return
    target = (sender, rest[-1])
    (directory / "target").write_text(" ".join(target))
    record(method)
    reply.return_value(None)


def invoke():
    pending = directory / "command"
    if not target or not pending.exists():
        return True
    method, cookie = pending.read_text().split()
    pending.unlink()
    if method == "BeginAuthentication":
        identities = [
            ("unix-user", {"uid": GLib.Variant("u", uid)}) for uid in (0, 1000)
        ]
        args = GLib.Variant(
            "(sssa{ss}sa(sa{sv}))",
            (
                "org.joelinux.test",
                "Change test setting",
                "",
                {"device": "Virtual display"},
                cookie,
                identities,
            ),
        )
    else:
        args = GLib.Variant("(s)", (cookie,))

    def result(conn, res):
        try:
            conn.call_finish(res)
            status = "ok"
        except GLib.Error as error:
            status = Gio.DBusError.get_remote_error(error)
        record(f"{method} {cookie} {status}")

    bus.call(*target, agent, method, args, None, 0, -1, None, result)
    return True


bus.register_object(
    "/org/freedesktop/PolicyKit1/Authority",
    Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0],
    registration,
)
Gio.bus_own_name_on_connection(bus, name, 0, None, None)
GLib.timeout_add(30, invoke)
GLib.MainLoop().run()
