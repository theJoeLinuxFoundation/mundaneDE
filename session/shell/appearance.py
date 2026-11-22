"""Publish the desktop's appearance preferences and apply them to the shell."""

from fnmatch import fnmatchcase
import math
from gi.repository import Gio, GLib
from portals import FRONT, ROOT, IMPL, standard

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
        self.owner = None
        self.request = 0
        self.applied = None
        self.refresh()
        self.bus.signal_subscribe(
            FRONT,
            "org.freedesktop.portal.Settings",
            "SettingChanged",
            ROOT,
            APPEARANCE,
            Gio.DBusSignalFlags.NONE,
            self.changed,
        )
        Gio.bus_watch_name_on_connection(
            self.bus,
            FRONT,
            Gio.BusNameWatcherFlags.AUTO_START,
            self.appeared,
            self.vanished,
        )

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
        if self.owner is None:
            self.apply_local()

    def apply_local(self):
        self.apply({k: v.unpack() for k, v in self.values[APPEARANCE].items()})

    def appeared(self, bus, name, owner):
        self.owner = owner
        self.read_portal()

    def vanished(self, *_):
        self.owner = None
        self.apply_local()

    def changed(self, bus, sender, path, iface, signal, args):
        if sender == self.owner:
            self.read_portal()

    def read_portal(self):
        # The frontend may call our own backend to assemble the snapshot.
        # Read asynchronously and discard replies superseded by a newer owner or change.
        self.request += 1
        request, owner = self.request, self.owner

        def received(bus, task):
            try:
                (groups,) = bus.call_finish(task).unpack()
            except GLib.Error:
                return
            if request == self.request and owner == self.owner and APPEARANCE in groups:
                self.apply(groups[APPEARANCE])

        self.bus.call(
            owner,
            ROOT,
            "org.freedesktop.portal.Settings",
            "ReadAll",
            GLib.Variant("(as)", ([APPEARANCE],)),
            None,
            Gio.DBusCallFlags.NONE,
            5000,
            None,
            received,
        )

    def apply(self, values):
        light = values.get("color-scheme") != 1
        contrast = values.get("contrast") == 1
        reduce = values.get("reduced-motion") == 1
        rgb = values.get("accent-color", ())
        accent = ACCENTS["blue"]
        if len(rgb) == 3 and all(math.isfinite(c) and 0 <= c <= 1 for c in rgb):
            accent = "".join(f"{round(c * 255):02x}" for c in rgb)
        appearance = light, contrast, reduce, accent
        if appearance == self.applied:
            return
        self.applied = appearance
        self.panel.send("reduced-motion " + ("on" if reduce else "off"))
        fg, bg = ("#141414", "#fafafa") if light else ("#f1f4fb", "#1b2130")
        if contrast:
            fg, bg = ("#ffffff", "#000000")
        edge = fg if contrast else "#748094"
        surface = ("#ffffff" if light else "#2a3342") if not contrast else bg
        hover = ("#e6efec" if light else "#374452") if not contrast else bg
        selected = ("#d6ebe6" if light else "#24483f") if not contrast else "#ffffff"
        ink = ("#194a3c" if light else "#beeed7") if not contrast else "#000000"
        muted = ("#4d6060" if light else "#c2cbd0") if not contrast else fg
        transition = (
            "none"
            if reduce
            else "background-color 140ms ease, color 140ms ease, border-color 140ms ease"
        )
        self.css.load_from_data(
            (
                self.base
                + f"""
.bar,.menu,.toast {{background:{bg};color:{fg};border-color:{edge};}}
.bar button.start,.menu button:hover,.bar button:hover {{background:{bg};color:{fg};outline:1px solid {edge};}}
 .menu {{border-radius:16px;padding:16px;}}
 .menu-heading {{font-size:1.45em;font-weight:700;letter-spacing:-0.5px;}}
 .menu-caption,.tile-state,.level-value {{color:{muted};font-size:0.92em;}}
 .menu button.quick-tile,.menu button.desk-tile,.menu button.icon-command,.level-card {{background:{surface};border:1px solid {edge if contrast else ("#d2ded9" if light else "#465360")};border-radius:10px;}}
 .menu button.quick-tile,.menu button.desk-tile {{padding:10px;transition:{transition};}}
 .menu button.quick-tile:hover,.menu button.desk-tile:hover,.menu button.icon-command:hover {{background:{hover};outline:1px solid {edge};}}
 .menu button.quick-tile:checked,.menu button.desk-tile:checked {{background:{selected};color:{ink};border-color:{ink if contrast else ("#83b4a3" if light else "#609982")};}}
 .menu button.quick-tile:checked label,.menu button.desk-tile:checked label {{color:{ink};}}
 .menu button.quick-tile:active,.menu button.desk-tile:active,.menu button.icon-command:active,.bar button:active {{background:{ink};color:{selected};}}
 .menu button.quick-tile:active label,.menu button.desk-tile:active label {{color:{selected};}}
 .menu button.quick-tile:disabled {{opacity:0.72;}}
 .tile-name,.battery {{font-weight:600;}}
 .level-card {{padding:10px 12px;}}
 .level-card scale {{padding:5px 3px;min-height:14px;}}
 .level-card scale trough {{border-radius:6px;min-height:5px;background:{edge};}}
 .level-card scale highlight {{background:{"#ffff00" if contrast else "#" + accent};border-radius:6px;}}
 .menu button.icon-command {{padding:8px 5px;}}
 .menu .details {{padding:4px 0;}}
 .bar .desk-marker {{background:{surface};color:{muted};border:1px solid {edge};border-radius:6px;min-width:18px;padding:1px 2px;transition:{transition};}}
 .bar .desk-marker.occupied {{border-bottom:3px solid {fg};}}
 .bar .desk-marker.current {{background:{selected};color:{ink};border:2px solid {ink};font-weight:700;}}
 .bar button.running {{border-bottom:3px solid {ink if not contrast else fg};}}
 .switch-row {{padding:7px 10px;border-radius:7px;}}
 .switch-row.selected {{background:{selected};color:{ink};}}
 .desk-number {{font-size:1.4em;font-weight:700;}}
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
