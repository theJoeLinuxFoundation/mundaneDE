"""Observe insertions in the test field, and check a normal bus subscriber sees no key signals."""

import json
import os
from pathlib import Path
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib, Gio

runtime = Path(os.environ["XDG_RUNTIME_DIR"])
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def key(*args):
    (runtime / "unexpected-monitor-event").touch()


bus.signal_subscribe(
    None, "org.freedesktop.a11y.KeyboardMonitor", "KeyEvent", None, None, 0, key
)


def insert(event):
    if event.source.get_name() == "City":
        with (runtime / "echo-inserts").open("a") as stream:
            print(
                json.dumps([event.detail1, event.detail2, event.any_data]), file=stream
            )


listener = Atspi.EventListener.new(insert)
assert listener.register("object:text-changed:insert")
(runtime / "observer-ready").touch()
GLib.MainLoop().run()
