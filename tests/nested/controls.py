"""Exercise the title buttons GTK creates from the negotiated capabilities."""

import sys
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, GLib

Gtk.init()
Gtk.Settings.get_default().set_property("gtk-decoration-layout", ":minimize,maximize,close")
window = Gtk.Window(title="Native controls")
window.set_titlebar(Gtk.HeaderBar())
window.set_default_size(280, 150)
entry = Gtk.Entry()
window.set_child(entry)
window.present()
loop = GLib.MainLoop()


def descendants(widget):
    yield widget
    child = widget.get_first_child()
    while child:
        yield from descendants(child)
        child = child.get_next_sibling()


def command(_fd, _condition):
    action = sys.stdin.readline().strip()
    if action == "exit":
        loop.quit()
        return False
    if action.startswith("locate "):
        name = action.split()[1]
        button = next(
            w for w in descendants(window) if isinstance(w, Gtk.Button) and w.has_css_class(name)
        )
        assert button.get_mapped()
        ok, bounds = button.compute_bounds(window)
        assert ok
        print(
            "button",
            name,
            int(bounds.get_x() + bounds.get_width() / 2),
            int(bounds.get_y() + bounds.get_height() / 2),
            flush=True,
        )
    elif action.startswith("activate "):
        window.set_startup_id(action.split()[1])
        window.present()
    elif action == "focus":
        print("active", window.is_active(), flush=True)
    elif action == "unfullscreen":
        window.unfullscreen()
    elif action == "copy":
        window.get_clipboard().set("Wayland café")
    elif action == "paste":
        window.get_clipboard().read_text_async(
            None, lambda clipboard, res: print("pasted", clipboard.read_text_finish(res), flush=True)
        )
    else:
        raise ValueError(action)
    return True


GLib.io_add_watch(0, GLib.PRIORITY_DEFAULT, GLib.IO_IN, command)
loop.run()
