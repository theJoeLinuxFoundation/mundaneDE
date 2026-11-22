"""Ten real windows for the frame budget run. Remap one at a time to start its placement spring."""

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
windows = []
for i in range(10):
    window = Gtk.Window(title=f"Frame fixture {i}", decorated=False)
    window.set_default_size(230, 140)
    label = Gtk.Label(label=f"Window {i}")
    window.set_child(label)
    click = Gtk.GestureClick()

    def drag(gesture, count, x, y, window=window):
        event = gesture.get_current_event()
        window.get_surface().begin_move(event.get_device(), 1, x, y, event.get_time())

    click.connect("pressed", drag)
    window.add_controller(click)
    window.present()
    windows.append(window)

index = 0
running = True


def move():
    global index
    if running:
        window = windows[index % len(windows)]
        window.set_visible(False)
        GLib.idle_add(lambda: (window.present(), False)[1])
        index += 1
    return True


def command(*_):
    global running
    running = False
    return False


GLib.timeout_add(145, move)
GLib.io_add_watch(0, GLib.PRIORITY_DEFAULT, GLib.IO_IN, command)
GLib.MainLoop().run()
