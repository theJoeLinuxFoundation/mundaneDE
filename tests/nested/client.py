"""A real GTK Wayland toplevel with fixed, observable buffer contents."""
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, GLib, Gtk

Gtk.init()
window = Gtk.Window(decorated=False, title="mundane nested test")
css = Gtk.CssProvider()
css.load_from_string("window { background: #20df61; border-radius: 0; box-shadow: none; }")
Gtk.StyleContext.add_provider_for_display(
    Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
)
window.set_default_size(120, 80)
loop = GLib.MainLoop()


def command(_fd, _condition):
    if sys.stdin.readline().strip() == "resize":
        window.set_default_size(200, 110)
        return GLib.SOURCE_CONTINUE
    window.destroy()
    loop.quit()
    return GLib.SOURCE_REMOVE


GLib.io_add_watch(0, GLib.PRIORITY_DEFAULT, GLib.IO_IN | GLib.IO_HUP, command)
window.present()
loop.run()
