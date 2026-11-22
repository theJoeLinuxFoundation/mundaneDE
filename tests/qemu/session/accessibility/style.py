"""Read the active libadwaita style in one long-lived native client."""

import gi
import json
import os
from pathlib import Path

gi.require_version("Adw", "1")
from gi.repository import Adw, GLib, Gtk

Adw.init()
style = Adw.StyleManager.get_default()
window = Adw.Window(title="Native appearance test")
window.set_default_size(280, 180)
window.set_content(Gtk.Label(label="Native appearance"))
window.present()


def snapshot(*args):
    data = dict(
        dark=style.get_dark(),
        contrast=style.get_high_contrast(),
        accent=style.get_accent_color().value_nick,
        animations=Gtk.Settings.get_default().get_property("gtk-enable-animations"),
    )
    dest = Path(os.environ["XDG_RUNTIME_DIR"]) / "gtk-style"
    tmp = dest.with_suffix(".tmp")
    tmp.write_text(json.dumps(data))
    tmp.replace(dest)
    return True


GLib.timeout_add(100, snapshot)
GLib.MainLoop().run()
