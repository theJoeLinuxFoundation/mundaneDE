"""Publish the desktop's appearance preferences and apply them to the shell."""

from fnmatch import fnmatchcase
from gi.repository import Gio, GLib
from portals import ROOT, IMPL, standard

APPEARANCE = "org.freedesktop.appearance"
INTERFACE = "org.gnome.desktop.interface"
A11Y = "org.gnome.desktop.a11y.interface"
# Libadwaita's named accent colours, in sRGB.
ACCENTS = dict(
    blue="3584e4",
    teal="2190a4",
    green="3a944a",
    yellow="c88800",
    orange="ed5b00",
    red="e62d42",
    pink="d56199",
    purple="9141ac",
    slate="6f8396",
)


class Appearance:
    def __init__(self, panel, css, base):
        self.panel, self.css, self.base = panel, css, base.decode()
        self.prefs = Gio.Settings.new(INTERFACE)
        self.a11y = Gio.Settings.new(A11Y)
        self.bus = panel.bus.bus
        self.bus.register_object(
            ROOT, standard("Settings"), self.method, lambda *_: GLib.Variant("u", 1)
        )
        self.values = {}
        self.prefs.connect("changed", self.refresh)
        self.a11y.connect("changed", self.refresh)
        self.refresh()

    def refresh(self, *_):
        scheme = self.prefs.get_string("color-scheme")
        accent = ACCENTS.get(self.prefs.get_string("accent-color"), ACCENTS["blue"])
        contrast = self.a11y.get_boolean("high-contrast")
        reduce = not self.prefs.get_boolean("enable-animations")
        colors = tuple(int(accent[i : i + 2], 16) / 255 for i in (0, 2, 4))
        values = {
            APPEARANCE: {
                "color-scheme": GLib.Variant(
                    "u", {"prefer-dark": 1, "prefer-light": 2}.get(scheme, 0)
                ),
                "accent-color": GLib.Variant("(ddd)", colors),
                "contrast": GLib.Variant("u", int(contrast)),
                "reduced-motion": GLib.Variant("u", int(reduce)),
            },
            INTERFACE: {
                key: self.prefs.get_value(key)
                for key in (
                    "font-name",
                    "text-scaling-factor",
                    "enable-animations",
                    "color-scheme",
                    "accent-color",
                )
            },
        }
        for group, keys in values.items():
            for key, value in keys.items():
                if value != self.values.get(group, {}).get(key):
                    self.bus.emit_signal(
                        None,
                        ROOT,
                        IMPL + "Settings",
                        "SettingChanged",
                        GLib.Variant("(ssv)", (group, key, value)),
                    )
        self.values = values
        self.panel.send("reduced-motion " + ("on" if reduce else "off"))
        light = scheme != "prefer-dark"
        fg, bg = ("#141414", "#fafafa") if light else ("#f1f4fb", "#1b2130")
        if contrast:
            fg, bg = ("#ffffff", "#000000")
        edge = fg if contrast else "#748094"
        self.css.load_from_data(
            (
                self.base
                + f"""
.bar,.menu,.toast {{background:{bg};color:{fg};border-color:{edge};}}
.bar button.start,.menu button:hover,.bar button:hover {{background:{bg};color:{fg};outline:1px solid {edge};}}
button:focus-visible,entry:focus-visible {{outline:3px solid {"#ffff00" if contrast else "#" + accent};outline-offset:-3px;}}
"""
            ).encode()
        )

    def method(self, bus, sender, path, iface, method, args, reply):
        if method == "ReadAll":
            patterns = args.unpack()[0]
            values = {
                name: keys
                for name, keys in self.values.items()
                if not patterns or any(fnmatchcase(name, p) for p in patterns)
            }
            reply.return_value(GLib.Variant("(a{sa{sv}})", (values,)))
        else:
            name, key = args.unpack()
            value = self.values.get(name, {}).get(key)
            if value is None:
                reply.return_dbus_error(
                    "org.freedesktop.portal.Error.NotFound", "Unknown setting"
                )
            else:
                reply.return_value(GLib.Variant("(v)", (value,)))
