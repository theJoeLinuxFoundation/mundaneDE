"""A GTK EGL surface with visible fiducials, a submenu and keyboard/pointer reporting."""

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gtk, Gdk, GLib, Graphene

Gtk.init()
win = Gtk.Window(title="Output test", decorated=False)


def rectangle(x, y, w, h):
    rect = Graphene.Rect()
    rect.init(x, y, w, h)
    return rect


def color(value):
    c = Gdk.RGBA()
    c.parse(value)
    return c


class Sample(Gtk.Widget):
    def do_snapshot(self, snapshot):
        snapshot.append_color(color("#20df61"), rectangle(0, 0, 200, 120))
        scale = win.get_surface().get_scale()
        for i in range(0, 32, 2):
            snapshot.append_color(color("black"), rectangle(24 + i / scale, 50, 1 / scale, 8))


area = Sample()
area.set_size_request(200, 120)
overlay = Gtk.Overlay()
overlay.set_child(area)
text = Gtk.Label(label="Display 123")
text.set_halign(Gtk.Align.START)
text.set_valign(Gtk.Align.START)
text.set_can_target(False)
overlay.add_overlay(text)
style = Gtk.CssProvider()
style.load_from_string("popover contents {background:#db238a;} label {color:black;}")
Gtk.StyleContext.add_provider_for_display(
    Gdk.Display.get_default(), style, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
)
click = Gtk.GestureClick()
click.set_button(0)
pop = Gtk.Popover()
pop.set_child(Gtk.Label(label="Edge menu"))
pop.set_parent(area)
pop.set_position(Gtk.PositionType.RIGHT)


def press(gesture, _, x, y):
    print(f"hit {x:.1f} {y:.1f}", flush=True)
    if gesture.get_current_button() == 3:
        r = Gdk.Rectangle()
        r.x = int(x)
        r.y = int(y)
        r.width = r.height = 1
        pop.set_pointing_to(r)
        pop.popup()


click.connect("pressed", press)
area.add_controller(click)
key = Gtk.EventControllerKey()
key.connect("key-pressed", lambda _o, k, *_: print("key", k, flush=True) or False)
win.add_controller(key)
win.set_child(overlay)
win.present()
win.get_surface().connect("notify::scale", lambda *_: area.queue_draw())
GLib.MainLoop().run()
