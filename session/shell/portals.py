"""ScreenCast and Screenshot consent, session ownership and shell stop controls."""

import array
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
from dataclasses import dataclass

from gi.repository import Gio, GLib

ROOT = "/org/freedesktop/portal/desktop"
FRONT = "org.freedesktop.portal.Desktop"
IMPL = "org.freedesktop.impl.portal."
NAME = IMPL + "desktop.mundane"
MARKER = "org.joelinux.mundane.ActiveShare"
MANAGER = "org.freedesktop.DBus.ObjectManager"


def standard(name):
    filename = IMPL + name + ".xml"
    for base in GLib.get_system_data_dirs():
        path = Path(base) / "dbus-1/interfaces" / filename
        if path.exists():
            return Gio.DBusNodeInfo.new_for_xml(path.read_text()).interfaces[0]
    raise FileNotFoundError(filename)


def interface(xml):
    return Gio.DBusNodeInfo.new_for_xml("<node>" + xml + "</node>").interfaces[0]


@dataclass
class Session:
    path: str
    registration: int
    types: int = 0
    cursor: int = 1
    source: str = ""
    worker: subprocess.Popen | None = None
    marker: int = 0


class CapturePortals:
    def __init__(self, panel):
        self.panel = panel
        self.bus = panel.bus.bus
        self.sessions = {}
        self.requests = {}
        self.choosing = None
        self.locked = False
        self.front = None
        self.marker_info = interface(
            f'<interface name="{MARKER}"><property name="Source" type="s" access="read"/></interface>'
        )
        self.session_info = standard("Session")
        self.request_info = standard("Request")
        for name in ("ScreenCast", "Screenshot"):
            self.bus.register_object(ROOT, standard(name), self.method, self.prop, None)
        manager = interface(f'''<interface name="{MANAGER}">
        <method name="GetManagedObjects"><arg type="a{{oa{{sa{{sv}}}}}}" direction="out"/></method>
        <signal name="InterfacesAdded"><arg type="o"/><arg type="a{{sa{{sv}}}}"/></signal>
        <signal name="InterfacesRemoved"><arg type="o"/><arg type="as"/></signal></interface>''')
        self.bus.register_object(ROOT, manager, self.method, None, None)
        Gio.bus_own_name_on_connection(
            self.bus, NAME, Gio.BusNameOwnerFlags.NONE, None, None
        )
        Gio.bus_watch_name_on_connection(
            self.bus, FRONT, Gio.BusNameWatcherFlags.NONE, self.appeared, self.vanished
        )

    def appeared(self, bus, name, owner):
        self.front = owner

    def vanished(self, *_):
        self.front = None
        self.cancel_all()

    def cancel_all(self):
        for h in list(self.requests):
            self.finish(h, 1)
        for path in list(self.sessions):
            self.close(path)

    def prop(self, b, s, path, iface, key):
        values = {
            "AvailableSourceTypes": 3,
            "AvailableTargets": 3,
            "AvailableCursorModes": 3,
            "version": 3
            if iface == IMPL + "ScreenCast"
            else 2
            if iface == IMPL + "Screenshot"
            else 1,
        }
        return GLib.Variant("u", values[key])

    def emit(self, path, iface, name, signature, *args):
        self.bus.emit_signal(None, path, iface, name, GLib.Variant(signature, args))

    def properties(self, s):
        return {MARKER: {"Source": GLib.Variant("s", s.source)}}

    def method(self, bus, sender, path, iface, method, args, reply):
        values = args.unpack()
        if method == "GetManagedObjects":
            items = {
                p: self.properties(s) for p, s in self.sessions.items() if s.marker
            }
            reply.return_value(GLib.Variant("(a{oa{sa{sv}}})", (items,)))
            return
        if sender != self.front and not (
            sender == bus.get_unique_name()
            and iface == IMPL + "Session"
            and method == "Close"
        ):
            reply.return_dbus_error(
                "org.freedesktop.DBus.Error.AccessDenied", "Portal frontend required"
            )
            return
        if method == "Close":
            if iface == IMPL + "Session":
                self.close(path)
            else:
                self.finish(path, 1)
            reply.return_value(None)
            return

        def respond(code, data=None):
            reply.return_value(GLib.Variant("(ua{sv})", (code, data or {})))

        if self.locked or not self.panel.outputs:
            respond(1)
            return
        if method == "CreateSession":
            h, p, app, opts = values
            if p in self.sessions:
                respond(2)
                return
            registration = bus.register_object(
                p, self.session_info, self.method, self.prop, None
            )
            self.sessions[p] = Session(p, registration)
            respond(0)
        elif method == "SelectSources":
            h, p, app, opts = values
            s = self.sessions.get(p)
            if not s or s.types:
                respond(2)
                return
            s.types = opts.get("types", 1) & 3
            s.cursor = opts.get("cursor_mode", 1)
            respond(0 if s.types and s.cursor in (1, 2) else 2)
        elif method in ("Start", "Screenshot"):
            h = values[0]
            s = self.sessions.get(values[1]) if method == "Start" else None
            if method == "Start" and (not s or not s.types or s.worker):
                respond(2)
                return
            if h in self.requests or self.choosing is not None:
                respond(1)
                return
            reg = bus.register_object(h, self.request_info, self.method, None, None)
            self.requests[h] = dict(
                reply=reply,
                reg=reg,
                session=s,
                process=None,
                file=None,
                watch=0,
                timeout=0,
            )
            self.choose(h, values[2] if s else values[1], s.types if s else 3)
        else:
            respond(2)

    def choose(self, h, app, types):
        share = self.requests[h]["session"] is not None
        heading = (app or "An application") + (
            " wants to share your screen." if share else " wants a screenshot."
        )
        choices = []
        if types & 1:
            choices.extend(
                ("output", m.get_connector(), m.get_connector())
                for m in self.panel.outputs
                if m.get_connector()
            )
        if types & 2:
            choices.extend(
                ("window", t["id"], t["title"] or t["app"]) for t in self.panel.tasks
            )
        self.panel.capture_chooser(
            heading,
            "Share" if share else "Take screenshot of",
            choices,
            lambda *choice: self.accept(h, *choice),
            lambda: self.finish(h, 1),
        )
        self.choosing = h

    def accept(self, h, kind, ident, label):
        if h not in self.requests or self.choosing != h or self.locked:
            return
        self.choosing = None
        self.panel.dismiss()
        request = self.requests[h]
        s = request["session"]
        try:
            with _grant(
                self.panel,
                "screenshot-output"
                if kind == "output" and not s
                else "capture-" + kind,
                ident,
            ) as conn:
                if s:
                    s.source = label
                    command = [
                        "dbus-run-session",
                        "--",
                        sys.executable,
                        str(Path(__file__).with_name("cast.py")),
                        kind,
                        ident,
                        str(s.cursor),
                    ]
                else:
                    fd, filename = tempfile.mkstemp(
                        prefix="mundane-screenshot-",
                        suffix=".png",
                        dir=GLib.get_user_runtime_dir(),
                    )
                    os.close(fd)
                    request["file"] = Path(filename)
                    command = [
                        "grim",
                        "-o" if kind == "output" else "-T",
                        ident,
                        filename,
                    ]
                p = subprocess.Popen(
                    command,
                    env={**os.environ, "WAYLAND_SOCKET": "0"},
                    stdin=conn,
                    stdout=subprocess.PIPE,
                    start_new_session=True,
                )
            request["process"] = p
            if s:
                s.worker = p
                request["watch"] = GLib.io_add_watch(
                    p.stdout.fileno(),
                    GLib.PRIORITY_DEFAULT,
                    GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR,
                    self.stream_ready,
                    h,
                    p,
                )

            def timed_out():
                request["timeout"] = 0
                self.finish(h, 2)
                return False

            request["timeout"] = GLib.timeout_add_seconds(20, timed_out)

            def exited(pid, status):
                p.returncode = os.waitstatus_to_exitcode(status)
                if s:
                    if self.sessions.get(s.path) is s:
                        self.close(s.path)
                elif h in self.requests:
                    image = request["file"]
                    if p.returncode == 0 and image.is_file() and image.stat().st_size:
                        self.finish(h, 0, {"uri": GLib.Variant("s", image.as_uri())})
                    else:
                        self.finish(h, 2)

            GLib.child_watch_add(GLib.PRIORITY_DEFAULT, p.pid, exited)
        except (OSError, ValueError) as err:
            print(f"Portal capture: {err}", file=sys.stderr)
            self.finish(h, 2)

    def stream_ready(self, fd, condition, h, p):
        if h not in self.requests:
            return False
        self.requests[h]["watch"] = 0
        line = p.stdout.readline(4096)
        p.stdout.close()
        try:
            node, props = json.loads(line)
            if not isinstance(node, int) or not 0 < node < 0xFFFFFFFF:
                raise ValueError("invalid PipeWire node")
            keys = {
                "source_type": "u",
                "pipewire-serial": "t",
                "size": "(ii)",
                "position": "(ii)",
                "mapping_id": "s",
            }
            metadata = {
                k: GLib.Variant(keys[k], v) for k, v in props.items() if k in keys
            }
            streams = GLib.Variant("a(ua{sv})", [(node, metadata)])
        except (ValueError, TypeError):
            self.finish(h, 2)
            return False
        s = self.requests[h]["session"]
        s.marker = self.bus.register_object(
            s.path, self.marker_info, None, lambda *_: GLib.Variant("s", s.source), None
        )
        self.emit(
            ROOT, MANAGER, "InterfacesAdded", "(oa{sa{sv}})", s.path, self.properties(s)
        )
        self.finish(h, 0, {"streams": streams})
        return False

    def finish(self, h, code, data=None):
        r = self.requests.pop(h, None)
        if not r:
            return
        if self.choosing == h:
            self.choosing = None
            self.panel.dismiss()
        for key in ("watch", "timeout"):
            if r[key]:
                GLib.source_remove(r[key])
        self.bus.unregister_object(r["reg"])
        if code:
            terminate(r["process"])
            if r["file"]:
                r["file"].unlink(missing_ok=True)
        r["reply"].return_value(GLib.Variant("(ua{sv})", (code, data or {})))

    def close(self, path):
        s = self.sessions.pop(path, None)
        if not s:
            return
        for h, r in list(self.requests.items()):
            if r["session"] is s:
                self.finish(h, 1)
        terminate(s.worker)
        if s.marker:
            self.bus.unregister_object(s.marker)
            self.emit(ROOT, MANAGER, "InterfacesRemoved", "(oas)", path, [MARKER])
        self.bus.unregister_object(s.registration)
        self.emit(path, IMPL + "Session", "Closed", "()")


def terminate(p):
    if p and p.returncode is None:
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass


def _grant(panel, command, ident):
    server, client = socket.socketpair()
    try:
        panel.channel.sendmsg(
            [f"{command} {ident}".encode()],
            [
                (
                    socket.SOL_SOCKET,
                    socket.SCM_RIGHTS,
                    array.array("i", [server.fileno()]),
                )
            ],
        )
    except BaseException:
        client.close()
        raise
    finally:
        server.close()
    return client
