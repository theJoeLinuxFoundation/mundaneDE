"""Two real GTK input contexts at distinct caret positions."""

import os
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

win = Gtk.Window(title="Input method test", default_width=560, default_height=360)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=160)
win.set_child(box)
runtime = Path(os.environ["XDG_RUNTIME_DIR"])
for label in ("First input", "Second input"):
    e = Gtk.Entry()
    e.update_property([Gtk.AccessibleProperty.LABEL], [label])
    e.connect(
        "changed",
        lambda entry, name=label: (runtime / name).write_text(entry.get_text()),
    )
    box.append(e)
win.present()
box.get_first_child().grab_focus()
GLib.MainLoop().run()
