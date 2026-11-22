"""The mundaneDE taskbar, launcher and system menu."""

import concurrent.futures
import datetime
import json
import os
from pathlib import Path
import socket
import subprocess

import gi

gi.require_version("Gdk", "4.0")
gi.require_version("Gtk", "4.0")
gi.require_version("Gtk4LayerShell", "1.0")
from gi.repository import Gdk, Gio, GLib, Gtk, Gtk4LayerShell as Layer
from bus import Services, Shares
from portals import CapturePortals
from bluetooth import Bluetooth
from devices import command, devices
from upgrades import UpgradeNotices

CSS = b"""
window { background: transparent; }
.bar, .menu, .toast { background: #1b2130; color: #f1f4fb; }
.bar { padding: 4px 9px; border-top: 1px solid #434a5d; }
.menu, .toast { padding: 14px; border: 1px solid #58617a; border-radius: 12px; }
button { border-radius: 7px; }
.bar button, .menu button, .desktop button { background: transparent; color: inherit; box-shadow: none; border: none; }
.bar button:hover, .menu button:hover, .desktop button:hover { background: #394155; }
button.start { background: #364761; padding-left: 14px; padding-right: 14px; font-weight: bold; }
.desktop { background: transparent; color: white; padding: 14px; }
.desktop button { text-shadow: 0 1px 3px black; }
.menu button { padding: 6px 9px; }
.menu entry { margin-bottom: 6px; }
"""


def button(label, fn, icon=None, desktop=False):
    widget = Gtk.Button()
    box = Gtk.Box(spacing=6, halign=Gtk.Align.CENTER)
    if desktop:
        box.set_orientation(Gtk.Orientation.VERTICAL)
    if icon:
        image = (
            Gtk.Image.new_from_gicon(icon)
            if not isinstance(icon, str)
            else Gtk.Image.new_from_icon_name(icon)
        )
        if desktop:
            image.set_pixel_size(40)
        box.append(image)
    if label:
        box.append(Gtk.Label(label=label, ellipsize=3, max_width_chars=32))
    widget.set_child(box)
    widget.set_tooltip_text(label)
    widget.connect("clicked", lambda *_: fn())
    return widget


def clear(box):
    while box.get_first_child():
        box.remove(box.get_first_child())


def vbox(spacing=5):
    return Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=spacing)


def scrolled(box, height):
    scroll = Gtk.ScrolledWindow(min_content_height=height)
    scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
    scroll.set_child(box)
    return scroll


class Panel:
    def __init__(self):
        self.channel = socket.socket(fileno=os.dup(1))
        os.dup2(2, 1)
        Gtk.init()
        self.display = Gdk.Display.get_default()
        os.environ.pop("WAYLAND_SOCKET", None)
        try:
            os.set_inheritable(0, False)
        except OSError:
            pass
        os.environ.pop("LD_PRELOAD", None)
        css = Gtk.CssProvider()
        css.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(
            self.display, css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )
        self.pool = concurrent.futures.ThreadPoolExecutor(max_workers=4)
        self.outputs = {}
        self.menu = None
        self.popup = None
        self.toast_id = None
        self.labels = {}
        self.toplevels = subprocess.Popen(
            ["mundane-de", "watch-toplevels"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )
        self.pool.submit(self.watch_toplevels)
        self.tasks = []
        self.workspace = 1
        self.devices = {}
        self.reading_devices = False
        self.reading_status = False
        self.page = None
        self.bus = Services(self)
        self.bluetooth = Bluetooth(self)
        self.shares = Shares(self.bus.bus, self.refresh_trays)
        self.upgrades = UpgradeNotices(
            Path(GLib.get_user_state_dir()) / "mundane" / "upgrade-notices.json"
        )
        self.apps = sorted(
            (a for a in Gio.AppInfo.get_all() if a.should_show()),
            key=lambda a: a.get_display_name().casefold(),
        )
        self.desktop = Path(
            GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_DESKTOP)
            or Path.home() / "Desktop"
        )
        self.monitor = None
        self.desktop.mkdir(exist_ok=True)
        self.monitor = Gio.File.new_for_path(str(self.desktop)).monitor_directory(
            Gio.FileMonitorFlags.NONE, None
        )
        self.monitor.connect("changed", lambda *_: self.icons())
        self.monitors = self.display.get_monitors()
        self.monitors.connect("items-changed", lambda *_: self.outputs_changed())
        self.outputs_changed()
        self.portals = CapturePortals(self)
        GLib.io_add_watch(
            self.channel.fileno(), GLib.PRIORITY_DEFAULT, GLib.IO_IN, self.receive
        )
        self.tick()
        self.poll_dave()
        GLib.timeout_add_seconds(1, self.tick)
        GLib.timeout_add_seconds(15, self.poll_dave)
        GLib.timeout_add_seconds(5, self.device_timer)

    def window(self, mon, namespace, layer, edges, keyboard=Layer.KeyboardMode.NONE):
        w = Gtk.Window(decorated=False)
        Layer.init_for_window(w)
        Layer.set_namespace(w, namespace)
        Layer.set_monitor(w, mon)
        Layer.set_layer(w, layer)
        Layer.set_keyboard_mode(w, keyboard)
        for e in edges:
            Layer.set_anchor(w, e, True)
        return w

    def outputs_changed(self):
        monitors = list(self.monitors)
        for m in list(self.outputs):
            if m not in monitors:
                bar, desk, *_ = self.outputs.pop(m)
                bar.destroy()
                desk.destroy()
                self.dismiss()
        for m in monitors:
            if m in self.outputs:
                continue
            e = Layer.Edge
            bar = self.window(
                m, "mundane-taskbar", Layer.Layer.TOP, [e.LEFT, e.BOTTOM, e.RIGHT]
            )
            Layer.auto_exclusive_zone_enable(bar)
            row = Gtk.Box(spacing=5)
            row.add_css_class("bar")
            start = button("Start", lambda m=m: self.start(m), "view-app-grid-symbolic")
            start.add_css_class("start")
            row.append(start)
            tasks = Gtk.Box(spacing=3, hexpand=True)
            tray = Gtk.Box(spacing=2)
            work = button(
                str(self.workspace), lambda m=m: self.overview(m), "view-grid-symbolic"
            )
            row.append(tasks)
            row.append(work)
            row.append(tray)
            clock = Gtk.Button()
            clock.set_child(Gtk.Label())
            clock.set_tooltip_text("System menu")
            clock.update_property([Gtk.AccessibleProperty.LABEL], ["System menu"])
            clock.connect("clicked", lambda _, m=m: self.system(m))
            row.append(clock)
            bar.set_child(row)
            desk = self.window(
                m,
                "mundane-desktop-icons",
                Layer.Layer.BOTTOM,
                [e.TOP, e.LEFT],
                Layer.KeyboardMode.ON_DEMAND,
            )
            icons = vbox()
            icons.add_css_class("desktop")
            desk.set_child(icons)
            self.outputs[m] = (bar, desk, tasks, tray, clock, work, icons)
            bar.present()
            desk.present()
        self.icons()
        self.draw_tasks()
        self.refresh_trays()

    def icons(self):
        for _, _, _, _, _, _, box in self.outputs.values():
            clear(box)
            for name, uri, icon in [
                ("Home", Path.home().as_uri(), "user-home"),
                ("Trash", "trash:///", "user-trash"),
            ]:
                box.append(
                    button(
                        name,
                        lambda u=uri: self.launch_file(Gio.File.new_for_uri(u)),
                        icon,
                        desktop=True,
                    )
                )
            for p in sorted(self.desktop.iterdir(), key=lambda p: p.name.casefold()):
                f = Gio.File.new_for_path(str(p))
                try:
                    icon = f.query_info("standard::icon", 0, None).get_icon()
                except GLib.Error:
                    continue
                b = button(p.name, lambda f=f: self.launch_file(f), icon, desktop=True)
                box.append(b)

    def launch_file(self, f):
        try:
            Gio.AppInfo.launch_default_for_uri(
                f.get_uri(), self.display.get_app_launch_context()
            )
        except GLib.Error as err:
            self.bus.notify("Could not open file", err.message)

    def send(self, command):
        self.channel.send(command.encode())

    def watch_toplevels(self):
        for line in self.toplevels.stdout:

            def update(entry):
                if "closed" in entry:
                    self.labels.pop(entry["closed"], None)
                else:
                    self.labels[entry["id"]] = entry
                self.send("list")
                return False

            GLib.idle_add(update, json.loads(line))

    def receive(self, *_):
        data = json.loads(self.channel.recv(262144))
        if data.get("open") == "start":
            if self.outputs:
                self.start(next(iter(self.outputs)))
        elif "desktop_icons" in data:
            self.dismiss()
            for monitor, (_, _, _, _, _, _, icons) in self.outputs.items():
                if monitor.get_connector() == data["desktop_icons"]:
                    first = icons.get_first_child()
                    if first:
                        first.grab_focus()
        elif "locked" in data:
            self.portals.locked = data["locked"]
            if self.portals.locked:
                self.portals.cancel_all()
        elif data.get("resume"):
            self.poll_dave()
        elif "osd" in data:
            self.volume(data["osd"])
        elif "workspace" in data:
            listed = [
                {**self.labels[t["id"]], **t}
                for t in data["tasks"]
                if t["id"] in self.labels
            ]
            tasks = sorted(listed, key=lambda t: (t["workspace"], t["title"], t["id"]))
            if (tasks, data["workspace"]) != (self.tasks, self.workspace):
                self.tasks, self.workspace = tasks, data["workspace"]
                self.draw_tasks()
        return True

    def tick(self):
        self.send("list")
        stamp = datetime.datetime.now().strftime("%H:%M")
        for _, _, _, _, clock, _, _ in self.outputs.values():
            clock.get_child().set_text(stamp + "  ▾")
        return True

    def draw_tasks(self):
        for _, _, box, _, _, work, _ in self.outputs.values():
            clear(box)
            work.set_tooltip_text(f"Workspace {self.workspace} · Overview")
            work.set_label(str(self.workspace))
            for t in self.tasks:
                if t["workspace"] != self.workspace:
                    continue
                try:
                    app = (
                        Gio.DesktopAppInfo.new(t["app"] + ".desktop")
                        if t["app"]
                        else None
                    )
                except TypeError:
                    app = None
                b = button(
                    t["title"] or t["app"],
                    lambda t=t: self.send("activate " + t["id"]),
                    app.get_icon() if app else "application-x-executable-symbolic",
                )
                if t["minimized"]:
                    b.set_opacity(0.7)
                box.append(b)

    def refresh_trays(self, *_):
        if not hasattr(self, "bus"):
            return False
        for _, _, _, tray, _, _, _ in self.outputs.values():
            clear(tray)
            if hasattr(self, "shares"):
                for path, source in self.shares.active.items():
                    tray.append(
                        button(
                            "Stop sharing " + source,
                            lambda p=path: self.shares.stop(p),
                            "media-record-symbolic",
                        )
                    )
            for dest, obj, _owner in self.bus.items.values():
                try:
                    props = self.bus.tray_info(dest, obj)
                except GLib.Error:
                    continue
                if props.get("Status") == "Passive":
                    continue
                b = button(
                    "",
                    lambda d=dest, o=obj: self.bus.activate_tray(d, o),
                    props.get("IconName") or "application-x-executable-symbolic",
                )
                title = props.get("Title") or props.get("Id") or dest
                b.set_tooltip_text(title)
                b.update_property([Gtk.AccessibleProperty.LABEL], [title])
                tray.append(b)
        return False

    def capture_chooser(self, heading, verb, choices, accept, cancel):
        box = self.open(next(iter(self.outputs)), "capture")
        box.append(Gtk.Label(label=heading, wrap=True))
        rows = vbox()
        box.append(scrolled(rows, 200))
        for choice in choices:
            rows.append(
                button(verb + " " + choice[2], lambda choice=choice: accept(*choice))
            )
        deny = button("Cancel", cancel)
        box.append(deny)
        self.menu.present()
        deny.grab_focus()

    def dismiss(self):
        if self.menu is not None:
            if self.page == "pairing":
                self.bluetooth.answer(False)
            old, self.menu = self.menu, None
            self.page = None
            old.destroy()
            if hasattr(self, "portals") and self.portals.choosing:
                self.portals.finish(self.portals.choosing, 1)

    def open(self, m, title, right=False):
        self.dismiss()
        w = self.window(
            m,
            "mundane-" + title,
            Layer.Layer.OVERLAY,
            [Layer.Edge.BOTTOM, Layer.Edge.RIGHT if right else Layer.Edge.LEFT],
            Layer.KeyboardMode.EXCLUSIVE,
        )
        Layer.set_margin(w, Layer.Edge.BOTTOM, 6)
        Layer.set_margin(w, Layer.Edge.RIGHT if right else Layer.Edge.LEFT, 8)
        outer = vbox()
        outer.add_css_class("menu")
        outer.set_size_request(370, -1)
        w.set_child(outer)
        self.menu = w
        self.page = title
        keys = Gtk.EventControllerKey()
        keys.connect("key-pressed", self.on_key)
        w.add_controller(keys)
        return outer

    def on_key(self, ctrl, key, code, mods):
        if key == Gdk.KEY_Escape:
            self.dismiss()
            return True
        return False

    def launch(self, app):
        context = self.display.get_app_launch_context()
        self.dismiss()
        try:
            app.launch([], context)
        except GLib.Error as err:
            self.bus.notify("Could not launch application", err.message)

    def start(self, m):
        box = self.open(m, "start")
        search = Gtk.SearchEntry(placeholder_text="Search applications")
        box.append(search)
        rows = vbox(1)
        box.append(scrolled(rows, 340))

        def populate(*_):
            clear(rows)
            query = search.get_text().casefold()
            for app in self.apps:
                if (
                    query
                    not in (
                        app.get_display_name() + " " + (app.get_id() or "")
                    ).casefold()
                ):
                    continue
                rows.append(
                    button(
                        app.get_display_name(),
                        lambda a=app: self.launch(a),
                        app.get_icon(),
                    )
                )

        populate()
        search.connect("search-changed", populate)
        search.connect(
            "activate",
            lambda *_: (
                rows.get_first_child() and rows.get_first_child().emit("clicked")
            ),
        )
        box.append(
            button("Window overview", lambda: self.overview(m), "view-grid-symbolic")
        )
        self.menu.present()
        search.grab_focus()

    def task_action(self, line):
        self.dismiss()
        self.send(line)

    def overview(self, m):
        box = self.open(m, "overview")
        tabs = Gtk.Box(spacing=5)
        for n in range(1, 5):
            tabs.append(
                button(f"Desk {n}", lambda n=n: self.task_action(f"workspace {n}"))
            )
        box.append(tabs)
        search = Gtk.SearchEntry(placeholder_text="Find a window")
        box.append(search)
        rows = vbox()
        box.append(scrolled(rows, 230))

        def populate(*_):
            clear(rows)
            for t in self.tasks:
                if (
                    search.get_text().casefold()
                    not in (t["title"] + " " + t["app"]).casefold()
                ):
                    continue
                r = Gtk.Box(spacing=3)
                r.append(
                    button(
                        t["title"] or t["app"],
                        lambda t=t: self.task_action("activate " + t["id"]),
                    )
                )
                r.append(
                    button(
                        "Move →",
                        lambda t=t: self.task_action(
                            f"move {t['id']} {t['workspace'] % 4 + 1}"
                        ),
                    )
                )
                r.append(button("×", lambda t=t: self.task_action("close " + t["id"])))
                rows.append(r)

        populate()
        search.connect("search-changed", populate)
        search.connect(
            "activate",
            lambda *_: (
                rows.get_first_child()
                and rows.get_first_child().get_first_child().emit("clicked")
            ),
        )
        self.menu.present()
        search.grab_focus()

    def background(self, fn, done, failed=None):
        def completed(result):
            try:
                value = result.result()
            except Exception as err:
                if failed:
                    GLib.idle_add(lambda e=str(err): failed(e) and False)
            else:
                GLib.idle_add(lambda: done(value) and False)

        self.pool.submit(fn).add_done_callback(completed)

    def poll_dave(self):
        if self.reading_status:
            return True
        self.reading_status = True

        def got(value):
            self.reading_status = False
            try:
                alerts = self.upgrades.update(value)
            except (KeyError, ValueError, TypeError):
                alerts = self.upgrades.update(None)
            for title, body in alerts:
                self.bus.notify(title, body)
            if self.page == "system":
                self.draw_system()

        self.background(
            lambda: json.loads(command("dave", "status", "--json")),
            got,
            lambda _: got(None),
        )
        return True

    def system(self, m):
        box = self.open(m, "system", True)
        self.system_rows = vbox()
        box.append(
            scrolled(
                self.system_rows, min(490, max(190, m.get_geometry().height - 115))
            )
        )
        self.system_monitor = m
        self.draw_system()
        self.menu.present()
        self.refresh_devices()

    def device_timer(self):
        if self.page == "system":
            self.refresh_devices()
        return True

    def refresh_devices(self):
        if self.reading_devices:
            return
        self.reading_devices = True

        def got(result):
            self.reading_devices = False
            if self.page == "system":
                if self.devices != result:
                    self.devices = result
                    self.draw_system()

        self.background(devices, got, lambda _: got({}))
        return False

    def run_action(self, *args):
        self.background(
            lambda: command(*args),
            lambda _: self.refresh_devices() if self.page == "system" else None,
            lambda e: self.bus.notify("Device action unavailable", e),
        )

    def volume(self, delta):
        def done(_):
            self.background(
                lambda: command("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"),
                lambda v: self.toast(None, "Volume", v, 1500),
            )

        self.background(
            lambda: command(
                "wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", delta
            ),
            done,
            lambda e: self.bus.notify("Audio unavailable", e),
        )

    def power(self, op):
        box = self.open(self.system_monitor, "power", True)
        box.append(Gtk.Label(label=f"{op.capitalize()} this machine?"))
        box.append(button(op.capitalize(), lambda: self.run_action("systemctl", op)))
        box.append(button("Cancel", self.dismiss))
        self.menu.present()

    def pairing(self, method, values, answer):
        box = self.open(self.system_monitor, "pairing", True)
        text = (
            "Confirm passkey %06d" % values[1]
            if method == "RequestConfirmation"
            else "Authorize Bluetooth pairing"
        )
        box.append(Gtk.Label(label=text))
        entry = None
        if method in ("RequestPinCode", "RequestPasskey"):
            entry = Gtk.Entry(placeholder_text="Enter the device passkey")
            box.append(entry)

        def accept():
            value = None
            if entry:
                text = entry.get_text()
                if method == "RequestPasskey":
                    if not text.isdigit() or len(text) > 6:
                        return
                    value = GLib.Variant("(u)", (int(text),))
                else:
                    value = GLib.Variant("(s)", (text,))
            answer(value)
            self.dismiss()

        box.append(button("Confirm", accept))
        box.append(button("Cancel", self.dismiss))
        self.menu.present()

    def draw_system(self):
        b = self.system_rows
        clear(b)

        def label(text):
            b.append(Gtk.Label(label=text, wrap=True, xalign=0))

        def action(name, fn):
            b.append(button(name, fn))

        for text in self.upgrades.details:
            label(text)
        d = self.devices
        label(d.get("battery", "Reading devices…"))
        label(d.get("volume") or "No audio output")
        row = Gtk.Box(spacing=4)
        row.append(button("Volume −", lambda: self.volume("5%-")))
        row.append(button("Volume +", lambda: self.volume("5%+")))
        b.append(row)
        for ident, name, current in d.get("sinks", []):
            action(
                ("✓ " if current else "") + name,
                lambda i=ident: self.run_action("wpctl", "set-default", i),
            )
        wifi = d.get("wifi")
        if wifi in ("enabled", "disabled"):
            action(
                "Wi-Fi " + wifi,
                lambda: self.run_action(
                    "nmcli", "radio", "wifi", "off" if wifi == "enabled" else "on"
                ),
            )
        for name in d.get("networks", []):
            action(
                name,
                lambda n=name: self.run_action("nmcli", "connection", "up", "id", n),
            )
        if d.get("adapter"):
            action(
                "Discover Bluetooth devices",
                lambda: self.run_action("bluetoothctl", "--timeout", "5", "scan", "on"),
            )
        elif d:
            label("No Bluetooth adapter")
        for addr, name in d.get("bluetooth", []):
            action("Pair " + name, lambda addr=addr: self.bluetooth.pair(addr))
            action(
                "Connect " + name,
                lambda addr=addr: self.run_action("bluetoothctl", "connect", addr),
            )
        for n, (title, body, sender, actions) in reversed(list(self.bus.notes.items())):
            label(title)
            if body:
                label(body)
            for key, name in actions.items():
                action(name, lambda i=n, k=key: self.bus.action(i, k))
            action("Dismiss", lambda i=n: self.bus.close(i, 2))
        for name, fn in [
            ("Lock", lambda: self.task_action("lock")),
            ("Suspend", lambda: self.power("suspend")),
            ("Restart", lambda: self.power("reboot")),
            ("Power off", lambda: self.power("poweroff")),
            ("Log out", lambda: self.task_action("logout")),
        ]:
            action(name, fn)

    def hide_toast(self, n):
        if self.popup and self.toast_id == n:
            self.popup.destroy()
            self.popup = None

    def toast(self, n, title, body, timeout):
        if not self.outputs:
            return
        if self.popup:
            self.popup.destroy()
        self.toast_id = n
        self.popup = self.window(
            next(iter(self.outputs)),
            "mundane-notification",
            Layer.Layer.OVERLAY,
            [Layer.Edge.TOP, Layer.Edge.RIGHT],
        )
        Layer.set_margin(self.popup, Layer.Edge.TOP, 12)
        Layer.set_margin(self.popup, Layer.Edge.RIGHT, 12)
        box = vbox()
        box.add_css_class("toast")
        box.append(Gtk.Label(label=title, xalign=0, max_width_chars=40, wrap=True))
        box.append(Gtk.Label(label=body, xalign=0, max_width_chars=40, wrap=True))
        if n in self.bus.notes:
            for key, name in self.bus.notes[n][3].items():
                box.append(button(name, lambda k=key: self.bus.action(n, k)))
        self.popup.set_child(box)
        self.popup.present()
        if timeout != 0:
            popup = self.popup

            def expire():
                if self.popup == popup:
                    self.hide_toast(n)
                return False

            GLib.timeout_add(max(1, timeout if timeout > 0 else 5000), expire)
        if self.page == "system":
            self.draw_system()


if __name__ == "__main__":
    panel = Panel()
    GLib.MainLoop().run()
