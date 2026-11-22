"""Run xdg-desktop-portal-wlr for one approved source on its own D-Bus session."""

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from gi.repository import Gio, GLib

BACKEND = "org.freedesktop.impl.portal.desktop.wlr"
ROOT = "/org/freedesktop/portal/desktop"
IFACE = "org.freedesktop.impl.portal.ScreenCast"


def main(kind, ident, cursor):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    path = shutil.which("xdg-desktop-portal-wlr") or next(
        (
            str(p)
            for p in (
                Path("/usr/libexec/xdg-desktop-portal-wlr"),
                Path("/usr/lib/xdg-desktop-portal-wlr"),
            )
            if p.is_file()
        ),
        None,
    )
    if not path:
        raise RuntimeError("Install xdg-desktop-portal-wlr")
    env = dict(
        os.environ,
        WAYLAND_SOCKET="0",
        MUNDANE_SELECTED_SOURCE=("Monitor: " if kind == "output" else "Window: ")
        + ident,
    )
    with tempfile.TemporaryDirectory(dir=os.environ["XDG_RUNTIME_DIR"]) as work:
        config = Path(work) / "config"
        # The command is constant. The compositor separately checks the source of every capture.
        config.write_text("""[screencast]
chooser_type=simple
chooser_cmd=printf '%s\\n' "$MUNDANE_SELECTED_SOURCE"
max_fps=30
""")
        child = subprocess.Popen(
            [path, "-c", config], stdin=sys.stdin, stdout=sys.stderr, env=env
        )
        try:
            for _ in range(100):
                if child.poll() is not None:
                    raise RuntimeError("wlr backend exited")
                if bus.call_sync(
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "NameHasOwner",
                    GLib.Variant("(s)", (BACKEND,)),
                    None,
                    0,
                    1000,
                    None,
                ).unpack()[0]:
                    break
                time.sleep(0.03)
            session = ROOT + "/session/mundane/cast"
            handle = ROOT + "/request/mundane/"

            def call(name, signature, args):
                code, data = bus.call_sync(
                    BACKEND,
                    ROOT,
                    IFACE,
                    name,
                    GLib.Variant(signature, args),
                    None,
                    0,
                    12000,
                    None,
                ).unpack()
                if code:
                    raise RuntimeError(f"{name} returned {code}")
                return data

            call("CreateSession", "(oosa{sv})", (handle + "create", session, "", {}))
            call(
                "SelectSources",
                "(oosa{sv})",
                (
                    handle + "select",
                    session,
                    "",
                    {
                        "types": GLib.Variant("u", 1 if kind == "output" else 2),
                        "cursor_mode": GLib.Variant("u", int(cursor)),
                    },
                ),
            )
            loop = GLib.MainLoop()
            bus.signal_subscribe(
                BACKEND,
                "org.freedesktop.impl.portal.Session",
                "Closed",
                session,
                None,
                0,
                lambda *_: loop.quit(),
            )

            def exited(pid, status):
                child.returncode = os.waitstatus_to_exitcode(status)
                loop.quit()

            GLib.child_watch_add(GLib.PRIORITY_DEFAULT, child.pid, exited)
            result = call(
                "Start", "(oossa{sv})", (handle + "start", session, "", "", {})
            )
            print(json.dumps(result["streams"][0]), flush=True)
            loop.run()
        finally:
            if child.poll() is None:
                child.terminate()
            child.wait()


if __name__ == "__main__":

    def terminate(*_):
        raise SystemExit(0)

    signal.signal(signal.SIGTERM, terminate)
    main(*sys.argv[1:])
