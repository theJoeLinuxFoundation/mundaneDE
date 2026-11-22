"""An edge-positioned menu containing a submenu and a transient dialog."""

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gtk, GLib

Gtk.init()
window = Gtk.Window(title="Popup tree")
window.set_decorated(False)
window.fullscreen()
style = Gtk.CssProvider()
colors = {"rootbutton": "#e4ae17", "submenu": "#aa3876", "dialogbutton": "#1959c4"}
style.load_from_string(
    "".join(f"#{name}, #{name} > button {{ background: {rgb}; }}" for name, rgb in colors.items())
)
Gtk.StyleContext.add_provider_for_display(
    window.get_display(), style, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
)
root = Gtk.MenuButton(label="Actions", name="rootbutton", halign=Gtk.Align.END, valign=Gtk.Align.END)
child = Gtk.MenuButton(label="More actions", name="submenu")
open_dialog = Gtk.Button(label="Open dialog", name="dialogbutton")
for button, content in [(root, child), (child, open_dialog)]:
    popover = Gtk.Popover()
    popover.set_child(content)
    button.set_popover(popover)


def dialog(_button):
    transient = Gtk.Window(title="Transient dialog", transient_for=window, modal=True)
    transient.set_child(Gtk.Label(label="Dialog contents"))
    transient.present()
    print("opened dialog", flush=True)


open_dialog.connect("clicked", dialog)
window.set_child(root)
window.present()
GLib.MainLoop().run()
