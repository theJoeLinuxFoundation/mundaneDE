"""Two monitors with distinct upper and lower colours distinguish source and orientation."""

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, Gdk, GLib

Gtk.init()
windows = []
for m in Gdk.Display.get_default().get_monitors():
    first = m.get_connector() == "Virtual-1"
    w = Gtk.Window(title="Pattern " + m.get_connector())
    area = Gtk.DrawingArea()

    def draw(area, cr, width, height, first=first):
        for n, c in enumerate(
            ((0.8, 0.1, 0.1), (0.55, 0.1, 0.1))
            if first
            else ((0.1, 0.7, 0.2), (0.1, 0.3, 0.8))
        ):
            cr.set_source_rgb(*c)
            cr.rectangle(0, n * height / 2, width, height / 2)
            cr.fill()

    area.set_draw_func(draw)
    w.set_child(area)
    w.present()
    w.fullscreen_on_monitor(m)
    windows.append(w)
GLib.MainLoop().run()
