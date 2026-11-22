"""Display StatusNotifierItem menus exported using com.canonical.dbusmenu."""

from gi.repository import Gio, GLib, Gtk

INTERFACE = "com.canonical.dbusmenu"


def show(panel, monitor, dest, path, anchor, title):
    if (
        panel.page == "tray"
        and panel.menu_monitor == monitor
        and panel.tray_item == (dest, path)
    ):
        panel.dismiss()
        return
    panel.tray_item = (dest, path)
    box = panel.open(monitor, "tray", True, anchor)
    panel.menu.set_title(title)
    box.append(Gtk.Label(label=title, xalign=0, css_classes=["menu-heading"]))
    rows = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
    scroll = Gtk.ScrolledWindow(
        propagate_natural_height=True,
        max_content_height=max(80, monitor.get_geometry().height - 180),
    )
    scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
    scroll.set_child(rows)
    box.append(scroll)
    stack = [0]

    def call(method, args, done=None):
        def result(bus, task):
            try:
                reply = bus.call_finish(task)
            except GLib.Error as error:
                if method != "AboutToShow" and panel.menu is window:
                    panel.bus.notify("Tray menu unavailable", error.message)
                reply = None
            if done and panel.menu is window:
                done(reply)

        panel.bus.bus.call(
            dest, path, INTERFACE, method, args, None, 0, 1500, None, result
        )

    def draw(reply):
        if not reply:
            return
        while rows.get_first_child():
            rows.remove(rows.get_first_child())
        if len(stack) > 1:
            back = Gtk.Button(label="Back")
            back.connect("clicked", lambda *_: (stack.pop(), refresh()))
            rows.append(back)
        _, (_, _, children) = reply.unpack()
        for item, props, _ in children:
            if not props.get("visible", True):
                continue
            if props.get("type") == "separator":
                rows.append(Gtk.Separator())
                continue
            name = (
                props.get("label", "")
                .replace("__", "\0")
                .replace("_", "")
                .replace("\0", "_")
            )
            submenu = props.get("children-display") == "submenu"
            control = Gtk.Button(label=name + ("  ›" if submenu else ""))
            if props.get("toggle-state") == 1:
                control.set_label("✓ " + name)
            control.update_property([Gtk.AccessibleProperty.LABEL], [name])
            control.set_sensitive(props.get("enabled", True))

            def activate(_, item=item, submenu=submenu):
                if submenu:
                    stack.append(item)
                    refresh()
                else:
                    call(
                        "Event",
                        GLib.Variant(
                            "(isvu)", (item, "clicked", GLib.Variant("i", 0), 0)
                        ),
                    )
                    panel.dismiss()

            control.connect("clicked", activate)
            rows.append(control)
        rows.child_focus(Gtk.DirectionType.TAB_FORWARD)

    def refresh(*_):
        call(
            "AboutToShow",
            GLib.Variant("(i)", (stack[-1],)),
            lambda _: call(
                "GetLayout", GLib.Variant("(iias)", (stack[-1], 1, [])), draw
            ),
        )

    window = panel.menu
    panel.tray_subscription = panel.bus.bus.signal_subscribe(
        dest, INTERFACE, "LayoutUpdated", path, None, Gio.DBusSignalFlags.NONE, refresh
    )
    window.present()
    refresh()
