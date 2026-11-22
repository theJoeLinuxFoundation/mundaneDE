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
from reader import Reader
from appearance import Appearance
import tray_menu
from bluetooth import Bluetooth
from quick_settings import QuickSettings, label as styled_label
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
        self.notifications = Gio.Settings.new("org.gnome.desktop.notifications")
        self.notifications.connect(
            "changed::show-banners",
            lambda *_: self.draw_system() if self.page == "system" else None,
        )
        self.tray_item = None
        self.tray_subscription = None
        self.menu_monitor = None
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
        self.appearance = Appearance(self, css, CSS)
        self.reader = Reader(self)
        self.auth = None
        if os.environ.get("XDG_SESSION_ID"):
            from authentication import Authentication

            self.auth = Authentication(
                self,
                Gio.bus_get_sync(Gio.BusType.SYSTEM, None),
                os.environ["XDG_SESSION_ID"],
            )
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
            work = Gtk.Button()
            markers = Gtk.Box(spacing=3)
            for n in range(1, 5):
                marker = Gtk.Label(label=str(n), css_classes=["desk-marker"])
                marker.set_tooltip_text(f"Workspace {n}")
                markers.append(marker)
            work.set_child(markers)
            work.connect("clicked", lambda _, m=m: self.overview(m))
            self.workspace_scroll(work)
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
        elif data.get("open") == "system" and self.outputs:
            monitor = next(
                (m for m in self.outputs if m.get_connector() == data.get("output")),
                next(iter(self.outputs)),
            )
            self.system(monitor)
        elif "desktop_icons" in data:
            self.dismiss()
            for monitor, (_, _, _, _, _, _, icons) in self.outputs.items():
                if monitor.get_connector() == data["desktop_icons"]:
                    first = icons.get_first_child()
                    if first:
                        first.grab_focus()
        elif data.get("dismiss_menu") and self.page in (
            "start",
            "system",
            "overview",
            "tray",
            "power",
        ):
            self.dismiss()
        elif "subscription" in data:
            self.reader.event(data)
        elif data.get("logout"):
            if self.auth:
                self.auth.close()
            if self.reader.child:
                self.reader.child.terminate()
            loop.quit()
            return False
        elif "screen_reader" in data:
            self.reader.toggle()
        elif "locked" in data:
            if self.auth:
                self.auth.locked = data["locked"]
                if self.auth.locked:
                    self.auth.finish(cancel=True)
            if self.outputs and self.reader.owner:
                bar = next(iter(self.outputs.values()))[0]
                hint = (
                    "Screen locked. Enter your password and press Enter."
                    if data["locked"]
                    else "Screen unlocked."
                )
                bar.announce(hint, Gtk.AccessibleAnnouncementPriority.HIGH)
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
            work.update_property(
                [Gtk.AccessibleProperty.LABEL],
                [f"Workspaces, current {self.workspace}"],
            )
            for n, marker in enumerate(work.get_child(), 1):
                count = sum(t["workspace"] == n for t in self.tasks)
                marker.set_css_classes(
                    ["desk-marker"]
                    + (["occupied"] if count else [])
                    + (["current"] if n == self.workspace else [])
                )
                marker.set_tooltip_text(
                    f"Workspace {n} · {count} windows"
                    + (" · Current" if n == self.workspace else "")
                )
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
        for monitor, (_, _, _, tray, _, _, _) in self.outputs.items():
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
                title = props.get("Title") or props.get("Id") or dest
                path = props.get("Menu")
                b = Gtk.Button(
                    child=Gtk.Image.new_from_icon_name(
                        props.get("IconName") or "application-x-executable-symbolic"
                    )
                )
                has_menu = bool(path and path != "/NO_DBUSMENU")
                if has_menu:

                    def open_tray(*_, btn=b, m=monitor, d=dest, p=path, t=title):
                        tray_menu.show(self, m, d, p, btn, t)

                    context = Gtk.GestureClick(button=3)
                    context.connect("released", open_tray)
                    b.add_controller(context)
                if has_menu and props.get("ItemIsMenu", False):
                    b.connect("clicked", open_tray)
                else:
                    b.connect(
                        "clicked", lambda _, d=dest, o=obj: self.bus.activate_tray(d, o)
                    )
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
            if self.page == "authentication" and self.auth.reply:
                self.auth.finish(cancel=True)
                return
            if self.page == "pairing":
                self.bluetooth.answer(False)
            if self.tray_subscription:
                self.bus.bus.signal_unsubscribe(self.tray_subscription)
                self.tray_subscription = None
            old, self.menu = self.menu, None
            self.page = None
            old.destroy()
            if hasattr(self, "portals") and self.portals.choosing:
                self.portals.finish(self.portals.choosing, 1)

    def toggled(self, m, page):
        if self.menu_monitor == m and self.page == page:
            self.dismiss()
            return True
        return False

    def open(self, m, title, right=False, anchor=None):
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
        w.set_child(outer)
        self.menu_monitor = m
        width = min(404, max(1, m.get_geometry().width - 16))
        outer.set_size_request(width, -1)
        outer.set_spacing(12)
        if anchor and right:
            ok, bounds = anchor.compute_bounds(self.outputs[m][0])
            if ok:
                Layer.set_margin(
                    w,
                    Layer.Edge.RIGHT,
                    max(
                        8,
                        min(
                            m.get_geometry().width - width - 8,
                            round(
                                m.get_geometry().width
                                - bounds.get_x()
                                - bounds.get_width()
                            ),
                        ),
                    ),
                )
        self.menu = w
        self.page = title
        keys = Gtk.EventControllerKey()
        keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
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
        if self.toggled(m, "start"):
            return
        box = self.open(m, "start")
        search = Gtk.SearchEntry(placeholder_text="Search applications")
        search.update_property([Gtk.AccessibleProperty.LABEL], ["Search applications"])
        self.menu.set_title("Start menu")
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

    def workspace_scroll(self, widget):
        flags = Gtk.EventControllerScrollFlags
        scroll = Gtk.EventControllerScroll.new(flags.BOTH_AXES | flags.DISCRETE)

        def switch(_ctrl, dx, dy):
            delta = dy or dx
            if delta:
                self.task_action(
                    "next-workspace" if delta > 0 else "previous-workspace"
                )
            return True

        scroll.connect("scroll", switch)
        widget.add_controller(scroll)

    def overview(self, m):
        if self.toggled(m, "overview"):
            return
        box = self.open(m, "overview", True, self.outputs[m][5])
        self.menu.set_title("Workspaces")
        box.append(styled_label("Workspaces", "menu-heading"))
        tabs = Gtk.Box(spacing=6, homogeneous=True)
        for n in range(1, 5):
            count = sum(t["workspace"] == n for t in self.tasks)
            occupancy = (
                f"{count} window" + ("" if count == 1 else "s") if count else "Empty"
            )
            status = (
                ("Current" if not count else f"Current · {count}")
                if n == self.workspace
                else occupancy
            )
            tile = Gtk.ToggleButton(
                active=n == self.workspace, css_classes=["desk-tile"]
            )
            inside = vbox(3)
            inside.append(styled_label(str(n), "desk-number"))
            inside.append(styled_label(status, "tile-state"))
            tile.set_child(inside)
            tile.set_tooltip_text(
                f"Workspace {n} · {occupancy}"
                + (" · Current" if n == self.workspace else "")
            )
            tile.update_property(
                [Gtk.AccessibleProperty.LABEL, Gtk.AccessibleProperty.DESCRIPTION],
                [f"Desk {n}", tile.get_tooltip_text()],
            )
            tile.connect("clicked", lambda _, n=n: self.task_action(f"workspace {n}"))
            tabs.append(tile)
        self.workspace_scroll(tabs)
        box.append(tabs)
        search = Gtk.SearchEntry(placeholder_text="Find a window")
        box.append(search)
        rows = vbox()
        scroll = Gtk.ScrolledWindow(
            min_content_height=40,
            max_content_height=min(230, max(50, m.get_geometry().height - 250)),
            propagate_natural_height=True,
        )
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.set_child(rows)
        box.append(scroll)

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
            if not rows.get_first_child():
                rows.append(
                    styled_label(
                        "No matching windows"
                        if search.get_text()
                        else "No open windows",
                        "menu-caption",
                    )
                )

        populate()
        search.connect("search-changed", populate)
        search.connect(
            "activate",
            lambda *_: (
                isinstance(rows.get_first_child(), Gtk.Box)
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
        if self.toggled(m, "system"):
            return
        outer = self.open(m, "system", True, self.outputs[m][4])
        self.menu.set_title("Quick Settings")
        self.system_monitor = m
        height = m.get_geometry().height - self.outputs[m][0].get_height() - 132
        self.quick = QuickSettings(self, outer, max(80, height))
        self.menu.present()
        self.quick.wifi.button.grab_focus()
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
        title = {"suspend": "Suspend", "reboot": "Restart", "poweroff": "Shut Down"}[op]
        box.append(styled_label(title + " this computer?", "menu-heading"))
        box.append(
            button(title, lambda: (self.dismiss(), self.run_action("systemctl", op)))
        )
        cancel = button("Cancel", self.dismiss)
        box.append(cancel)
        self.menu.present()
        cancel.grab_focus()

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
        if self.page == "system" and hasattr(self, "quick"):
            self.quick.refresh()

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
    loop = GLib.MainLoop()
    loop.run()
