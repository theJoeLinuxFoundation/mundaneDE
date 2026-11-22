"""Use the same focused text model for physical and application-driven insertions."""

import os
from pathlib import Path
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, GLib

runtime = Path(os.environ["XDG_RUNTIME_DIR"])
w = Gtk.Window(title="Echo test")
entry = Gtk.Entry(text="Tokyo")
entry.update_property([Gtk.AccessibleProperty.LABEL], ["City"])
w.set_child(entry)
w.present()
entry.grab_focus()
entry.set_position(-1)
entry.connect("changed", lambda e: (runtime / "echo-value").write_text(e.get_text()))


def inject():
    command = runtime / "append-character"
    if command.exists():
        command.unlink()
        entry.set_position(entry.insert_text("z", entry.get_position()))
    return True


GLib.timeout_add(50, inject)
GLib.MainLoop().run()
