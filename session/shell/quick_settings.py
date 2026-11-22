"""Quick settings controls on the session's active output."""

import datetime
import re
from gi.repository import GLib, Gtk


def label(text, style=None):
    widget = Gtk.Label(label=text, xalign=0, ellipsize=3)
    if style:
        widget.add_css_class(style)
    return widget


def image(name, size=18):
    icon = Gtk.Image.new_from_icon_name(name)
    icon.set_pixel_size(size)
    return icon


def named(widget, name):
    widget.set_tooltip_text(name)
    widget.update_property([Gtk.AccessibleProperty.LABEL], [name])


def icon_button(name, icon, callback):
    control = Gtk.Button(child=image(icon))
    control.add_css_class("icon-command")
    named(control, name)
    control.connect("clicked", lambda *_: callback())
    return control


class Tile:
    def __init__(self, name, icon, callback):
        self.name = name
        self.button = Gtk.ToggleButton(hexpand=True)
        self.button.add_css_class("quick-tile")
        row = Gtk.Box(spacing=10)
        self.icon = image(icon, 20)
        self.icon.add_css_class("tile-icon")
        row.append(self.icon)
        copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        copy.append(label(name, "tile-name"))
        self.state = label("…", "tile-state")
        copy.append(self.state)
        row.append(copy)
        self.button.set_child(row)
        self.button.connect("clicked", lambda *_: callback())

    def update(self, on, text, available=True):
        self.button.set_active(on)
        self.button.set_sensitive(available)
        self.state.set_text(text)
        named(self.button, f"{self.name}, {text}")


class Level:
    def __init__(self, name, icon, apply):
        self.box = Gtk.Box(spacing=10)
        self.box.add_css_class("level-row")
        self.box.append(image(icon))
        group = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, hexpand=True, spacing=3)
        heading = Gtk.Box()
        heading.append(label(name, "tile-name"))
        self.value = Gtk.Label(xalign=1, hexpand=True)
        self.value.add_css_class("level-value")
        heading.append(self.value)
        group.append(heading)
        self.scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 1)
        self.scale.set_draw_value(False)
        self.scale.set_increments(1, 10)
        named(self.scale, name)
        group.append(self.scale)
        self.box.append(group)
        self.pending = 0
        self.apply = apply
        self.handler = self.scale.connect("value-changed", self.change)

    def change(self, *_):
        value = round(self.scale.get_value())
        self.value.set_text(f"{value}%")
        if self.pending:
            GLib.source_remove(self.pending)

        def commit():
            self.pending = 0
            self.apply(value)
            return False

        self.pending = GLib.timeout_add(100, commit)

    def update(self, value):
        if self.pending:
            return
        self.scale.set_sensitive(value is not None)
        self.value.set_text(f"{value}%" if value is not None else "Unavailable")
        with self.scale.handler_block(self.handler):
            self.scale.set_value(value or 0)


def clear(box):
    while box.get_first_child():
        box.remove(box.get_first_child())


class QuickSettings:
    def __init__(self, panel, outer, height_limit):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.scroll = Gtk.ScrolledWindow(
            propagate_natural_height=True, max_content_height=height_limit
        )
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroll.set_child(box)
        outer.append(self.scroll)
        self.body = box
        self.panel = panel
        self.snapshot = None
        self.note_snapshot = None
        self.upgrade_snapshot = None
        self.device = {}
        box.add_css_class("quick-settings")
        header = Gtk.Box(spacing=12)
        title = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, hexpand=True)
        title.append(label("Quick Settings", "menu-heading"))
        title.append(
            label(datetime.datetime.now().strftime("%A, %B %-d"), "menu-caption")
        )
        header.append(title)
        self.battery = label("", "battery")
        header.append(self.battery)
        box.append(header)
        grid = Gtk.Grid(column_spacing=8, row_spacing=8, column_homogeneous=True)
        self.wifi = Tile(
            "Wi-Fi",
            "network-wireless-symbolic",
            lambda: panel.run_action(
                "nmcli",
                "radio",
                "wifi",
                "off" if self.device.get("wifi") == "enabled" else "on",
            ),
        )
        self.bluetooth = Tile(
            "Bluetooth",
            "bluetooth-symbolic",
            lambda: panel.run_action(
                "bluetoothctl", "power", "off" if self.device.get("powered") else "on"
            ),
        )
        self.quiet = Tile(
            "Do Not Disturb", "notifications-disabled-symbolic", self.toggle_quiet
        )
        for tile, x, y, w in [
            (self.wifi, 0, 0, 1),
            (self.bluetooth, 1, 0, 1),
            (self.quiet, 0, 1, 2),
        ]:
            grid.attach(tile.button, x, y, w, 1)
        box.append(grid)
        group = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        group.add_css_class("level-card")
        self.volume = Level(
            "Volume",
            "audio-volume-high-symbolic",
            lambda v: panel.run_action(
                "wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", f"{v}%"
            ),
        )
        self.brightness = Level(
            "Brightness", "display-brightness-symbolic", self.set_brightness
        )
        group.append(self.volume.box)
        group.append(self.brightness.box)
        box.append(group)
        self.devices = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        self.connections = Gtk.Expander(
            label="Devices and connections", child=self.devices
        )
        self.connections.add_css_class("details")
        box.append(self.connections)
        self.notices = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.history = Gtk.Expander(child=self.notices)
        self.history.add_css_class("details")
        box.append(self.history)
        self.upgrades = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        box.append(self.upgrades)
        outer.append(Gtk.Separator())
        power = Gtk.Box(spacing=6, homogeneous=True)
        for name, icon, fn in [
            ("Lock", "system-lock-screen-symbolic", lambda: panel.task_action("lock")),
            ("Log Out", "system-log-out-symbolic", lambda: panel.task_action("logout")),
            ("Suspend", "weather-clear-night-symbolic", lambda: panel.power("suspend")),
            ("Restart", "system-reboot-symbolic", lambda: panel.power("reboot")),
            ("Shut Down", "system-shutdown-symbolic", lambda: panel.power("poweroff")),
        ]:
            power.append(icon_button(name, icon, fn))
        outer.append(power)
        for expander in (self.connections, self.history):
            expander.connect_after("notify::expanded", self.fit)
        self.refresh()

    def fit(self, *_):
        # The layer surface must request a new height when async device rows
        # arrive. Otherwise GTK keeps its original scroll viewport at any scale.
        width = max(1, self.body.get_width() or 366)
        natural = self.body.measure(Gtk.Orientation.VERTICAL, width).natural
        self.scroll.set_min_content_height(
            min(natural, self.scroll.get_max_content_height())
        )

    def toggle_quiet(self):
        prefs = self.panel.notifications
        prefs.set_boolean("show-banners", not prefs.get_boolean("show-banners"))

    def set_brightness(self, percent):
        backlight = self.device.get("backlight")
        if backlight:
            name, maximum, _ = backlight
            self.panel.run_action(
                "busctl",
                "--system",
                "call",
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                "ssu",
                "backlight",
                name,
                str(round(maximum * percent / 100)),
            )

    def refresh(self):
        p = self.panel
        self.device = d = p.devices
        wifi = d.get("wifi")
        self.wifi.update(
            wifi == "enabled",
            {"enabled": "On", "disabled": "Off"}.get(wifi, "Unavailable"),
            wifi in ("enabled", "disabled"),
        )
        self.bluetooth.update(
            bool(d.get("powered")),
            ("On" if d.get("powered") else "Off") if d.get("adapter") else "No adapter",
            bool(d.get("adapter")),
        )
        quiet = not p.notifications.get_boolean("show-banners")
        self.quiet.update(quiet, "On" if quiet else "Off")
        volume = re.search(r"Volume:\s*([\d.]+)", d.get("volume", ""))
        self.volume.update(round(100 * float(volume[1])) if volume else None)
        if "MUTED" in d.get("volume", ""):
            self.volume.value.set_text("Muted")
        backlight = d.get("backlight")
        self.brightness.box.set_visible(bool(backlight))
        if backlight:
            self.brightness.update(backlight[2])
        battery = d.get("battery", "")
        self.battery.set_visible(battery.startswith("Battery ") and "%" in battery)
        self.battery.set_text(battery)
        details = (
            d.get("sinks"),
            d.get("networks"),
            d.get("adapter"),
            d.get("bluetooth"),
        )
        if self.snapshot != details:
            self.snapshot = details
            clear(self.devices)

            def action(box, name, fn):
                item = Gtk.Button(label=name)
                item.connect("clicked", lambda *_: fn())
                box.append(item)

            for ident, name, current in d.get("sinks", []):
                action(
                    self.devices,
                    ("✓ " if current else "") + name,
                    lambda i=ident: p.run_action("wpctl", "set-default", i),
                )
            for network in d.get("networks", []):
                action(
                    self.devices,
                    network,
                    lambda n=network: p.run_action(
                        "nmcli", "connection", "up", "id", n
                    ),
                )
            if d.get("adapter"):
                action(
                    self.devices,
                    "Discover Bluetooth devices",
                    lambda: p.run_action(
                        "bluetoothctl", "--timeout", "5", "scan", "on"
                    ),
                )
            for addr, name in d.get("bluetooth", []):
                action(self.devices, "Pair " + name, lambda a=addr: p.bluetooth.pair(a))
                action(
                    self.devices,
                    "Connect " + name,
                    lambda a=addr: p.run_action("bluetoothctl", "connect", a),
                )
            self.connections.set_visible(self.devices.get_first_child() is not None)
        notes = list(p.bus.notes.items())
        if notes != self.note_snapshot:
            self.note_snapshot = notes
            clear(self.notices)
            self.history.set_label(f"Notifications ({len(notes)})")
            self.history.set_visible(bool(notes))
            for n, (title, body, _, actions) in reversed(notes):
                self.notices.append(label(title, "tile-name"))
                if body:
                    self.notices.append(Gtk.Label(label=body, xalign=0, wrap=True))
                for key, name in actions.items():
                    item = Gtk.Button(label=name)
                    item.connect("clicked", lambda _, i=n, k=key: p.bus.action(i, k))
                    self.notices.append(item)
                item = Gtk.Button(label="Dismiss")
                item.connect("clicked", lambda _, i=n: p.bus.close(i, 2))
                self.notices.append(item)
        if self.upgrade_snapshot != p.upgrades.details:
            self.upgrade_snapshot = list(p.upgrades.details)
            clear(self.upgrades)
            for text in self.upgrade_snapshot:
                self.upgrades.append(
                    Gtk.Label(
                        label=text, xalign=0, wrap=True, css_classes=["menu-caption"]
                    )
                )
            self.upgrades.set_visible(bool(self.upgrade_snapshot))
        self.fit()
