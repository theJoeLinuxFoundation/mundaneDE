"""Exercise the shell agent against a disposable authority and helper conversation."""

import os
import subprocess
import sys
from types import SimpleNamespace
from pathlib import Path
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib, Gtk

sys.path.insert(0, "/usr/local/share/mundane/shell")
from authentication import AUTH, AUTH_PATH, AGENT_PATH, IFACE, Authentication

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
loop = GLib.MainLoop()
audit = Path(sys.argv[2])


def record(event):
    with audit.open("a") as log:
        print(event, file=log)


if sys.argv[1] == "agent":

    class Conversation:
        def __init__(self, user, cookie):
            self.uid = user.get_uid()
            self.cookie = cookie
            self.handlers = {}

        def connect(self, name, cb):
            self.handlers[name] = cb

        def initiate(self):
            record(f"begin {self.cookie} {self.uid}")
            self.handlers["request"](self, "Password:", False)

        def response(self, value):
            record("respond " + self.cookie)
            self.handlers["completed"](self, value == "fixture-correct")

        def cancel(self):
            record("cancel " + self.cookie)

    class Panel:
        outputs = [1]
        reader = SimpleNamespace(sensitive=False, configure=lambda: None)
        bus = SimpleNamespace(notify=lambda *args: record("denied"))

        def open(self, m, page):
            self.page = page
            self.menu = Gtk.Window()
            self.box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            self.menu.set_child(self.box)
            return self.box

        def dismiss(self):
            self.page = None
            self.menu.destroy()

    panel = Panel()
    agent = Authentication(panel, bus, "test", Conversation)

    def operate():
        command = audit.with_suffix(".command")
        if command.exists():
            op = command.read_text()
            command.unlink()
            if op == "quit":
                agent.close()
                loop.quit()
            elif op == "lock":
                agent.locked = True
                agent.finish(cancel=True)
            elif op == "cancel":
                agent.finish(cancel=True)
            elif op == "switch":
                choices = next(c for c in panel.box if isinstance(c, Gtk.DropDown))
                choices.set_selected(1)
            else:
                assert agent.entry and not agent.entry.get_visibility()
                agent.entry.set_text(op)
                agent.submit()
        return True

    GLib.timeout_add(20, operate)
    loop.run()
    sys.exit()

assert not os.environ.get("DBUS_SESSION_BUS_ADDRESS", "").endswith("/bus")
xml = f"""<node><interface name="{AUTH}.Authority">
<method name="RegisterAuthenticationAgent"><arg type="(sa{{sv}})"/><arg type="s"/><arg type="s"/></method>
<method name="UnregisterAuthenticationAgent"><arg type="(sa{{sv}})"/><arg type="s"/></method>
</interface></node>"""
owner = None


def registration(bus, sender, path, iface, method, args, reply):
    global owner
    assert args.unpack()[0] == ("unix-session", {"session-id": "test"})
    if method.startswith("Register"):
        owner = sender
    else:
        owner = None
    reply.return_value(None)


bus.register_object(
    AUTH_PATH, Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0], registration
)
Gio.bus_own_name_on_connection(bus, AUTH, 0, None, None)
proc = subprocess.Popen([sys.executable, __file__, "agent", str(audit)])
import time


def wait(fn):
    end = time.monotonic() + 8
    ctx = GLib.MainContext.default()
    while time.monotonic() < end:
        while ctx.pending():
            ctx.iteration(False)
        if fn():
            return
        time.sleep(0.015)
    raise AssertionError(audit.read_text() if audit.exists() else "unregistered")


def op(value):
    pending = audit.with_suffix(".tmp")
    pending.write_text(value)
    pending.replace(audit.with_suffix(".command"))


def invoke(method, args, results):
    def done(b, res):
        try:
            b.call_finish(res)
            results.append("success")
        except GLib.Error as error:
            results.append(Gio.DBusError.get_remote_error(error))

    bus.call(owner, AGENT_PATH, IFACE, method, args, None, 0, 10000, None, done)


try:
    wait(lambda: owner)
    for index, action in enumerate(
        ["switch", "wrong", "cancel", "authority", "lock", "locked"]
    ):
        result = []
        cookie = f"trial-{index}"
        identities = [
            ("unix-user", {"uid": GLib.Variant("u", uid)}) for uid in (0, 1000)
        ]
        invoke(
            "BeginAuthentication",
            GLib.Variant(
                "(sssa{ss}sa(sa{sv}))",
                (
                    "test.action",
                    "Change timezone",
                    "",
                    {"detail": "test"},
                    cookie,
                    identities,
                ),
            ),
            result,
        )
        if action != "locked":
            wait(lambda: audit.exists() and f"begin {cookie} 0" in audit.read_text())
            if action == "switch":
                op(action)
                wait(lambda: f"begin {cookie} 1000" in audit.read_text())
                op("fixture-correct")
            elif action == "authority":
                cancelled = []
                invoke(
                    "CancelAuthentication", GLib.Variant("(s)", (cookie,)), cancelled
                )
                wait(lambda: cancelled == ["success"])
            else:
                op(action)
        wait(lambda: len(result) > 0)
        assert result == (
            ["success"]
            if action in ("switch", "wrong")
            else [AUTH + ".Error.Cancelled"]
        ), (action, result)
        if action == "wrong":
            assert "denied" in audit.read_text()
        if action not in ("switch", "wrong"):
            assert "respond " + cookie not in audit.read_text()
    op("quit")
    wait(lambda: owner is None)
    print(
        "Two identities, masked input, success, denial, both cancellations, locked request and unregistration passed."
    )
finally:
    if proc.poll() is None:
        proc.terminate()
    proc.wait()
