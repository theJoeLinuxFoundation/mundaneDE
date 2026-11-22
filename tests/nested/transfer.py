"""Visible, distinct source and destination windows for seat-driven transfers."""

import sys
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gtk, GLib

Gtk.init()
source = sys.argv[1] == "source"
color = "#12a76d" if source else "#315cef"
window = Gtk.Window(title=sys.argv[1], decorated=False)
window.set_default_size(100, 60)
css = Gtk.CssProvider()
css.load_from_string(
    "window, label, entry, text {background: " + color + "; border-radius: 0; box-shadow: none;}"
)
Gtk.StyleContext.add_provider_for_display(
    window.get_display(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
)
if source:
    widget = Gtk.Label(label="Drag")
    gesture = Gtk.GestureClick()
    gesture.connect("pressed", lambda *args: window.get_primary_clipboard().set("primary text"))
    widget.add_controller(gesture)
    drag = Gtk.DragSource(actions=Gdk.DragAction.COPY)
    drag.set_content(Gdk.ContentProvider.new_for_value("dropped text"))
    widget.add_controller(drag)
else:
    widget = Gtk.Entry()
    widget.set_width_chars(4)
    widget.connect("changed", lambda entry: print("entry", entry.get_text(), flush=True))
    drop = Gtk.DropTarget.new(str, Gdk.DragAction.COPY)

    def dropped(target, value, x, y):
        print("received", value, flush=True)
        return True

    drop.connect("drop", dropped)
    widget.add_controller(drop)
widget.set_hexpand(True)
widget.set_vexpand(True)
window.set_child(widget)
window.present()
GLib.MainLoop().run()
