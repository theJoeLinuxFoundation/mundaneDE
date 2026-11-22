#!/usr/bin/python3
"""A D-Bus-activated GTK client for the joe session qualification."""

import os
from pathlib import Path
import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, Gtk

runtime = Path(os.environ["XDG_RUNTIME_DIR"])
events = runtime / "mundane-login-test.log"


def record(message):
    with events.open("a") as stream:
        print(message, file=stream)


class Probe(Gtk.Application):
    def do_activate(self):
        if self.get_active_window():
            self.get_active_window().present()
            return
        display = Gdk.Display.get_default()
        window = Gtk.ApplicationWindow(application=self, title="mundane session probe")
        window.set_decorated(False)
        window.set_default_size(300, 160)
        self.style = Gtk.CssProvider()
        self.style.load_from_string("window { background: #20df61; }")
        Gtk.StyleContext.add_provider_for_display(display, self.style, 800)
        key = Gtk.EventControllerKey()
        key.connect("key-pressed", self.key)
        window.add_controller(key)
        click = Gtk.GestureClick()
        click.connect("pressed", lambda *_: record("click"))
        window.add_controller(click)
        window.present()
        record(
            f"ready {os.getpid()} {os.getuid()} {display.get_name()} {os.environ['XDG_SESSION_ID']}"
        )

    def key(self, _, value, *_args):
        name = Gdk.keyval_name(value)
        record("key " + name)
        colors = {"a": "2088df", "b": "df8520", "c": "c12387", "d": "7820df"}
        if name in colors:
            self.style.load_from_string("window { background: #" + colors[name] + "; }")
        return False


Probe(application_id="org.joelinux.MundaneSessionTest", flags=Gio.ApplicationFlags.IS_SERVICE).run()
