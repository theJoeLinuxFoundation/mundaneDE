"""Inspect or activate a GTK shell control in the test guest via AT-SPI."""

import sys
import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib


def walk(node, depth=0):
    if node is None or depth > 18:
        return
    try:
        yield node
        for i in range(node.get_child_count()):
            yield from walk(node.get_child_at_index(i), depth + 1)
    except Exception:
        return


verb, *names = sys.argv[1:]
for node in walk(Atspi.get_desktop(0)):
    try:
        name = node.get_name()
        role = node.get_role_name()
    except GLib.Error:
        continue  # The inspected application just closed this object.
    if verb == "list":
        if name:
            print(role, name)
    elif (
        name == " ".join(names)
        and verb == "bar-point"
        and "Component" in node.get_interfaces()
    ):
        rect = node.get_extents(Atspi.CoordType.WINDOW)
        top = node
        while (
            top.get_parent() and top.get_parent().get_role() != Atspi.Role.APPLICATION
        ):
            top = top.get_parent()
        frame = top.get_extents(Atspi.CoordType.WINDOW)
        print(rect.x + rect.width // 2, frame.height - rect.y - rect.height // 2)
        sys.exit(0)
    elif name == " ".join(names) and verb == "bounds" and role == "button":
        rect = node.get_extents(Atspi.CoordType.WINDOW)
        print(rect.x, rect.y, rect.width, rect.height)
        sys.exit(0)
    elif name == " ".join(names) and verb == "size" and role == "frame":
        rect = node.get_extents(Atspi.CoordType.WINDOW)
        print(rect.width, rect.height)
        sys.exit(0)
    elif (
        name == " ".join(names)
        and verb == "focus"
        and node.get_state_set().contains(Atspi.StateType.FOCUSABLE)
    ):
        try:
            if node.grab_focus():
                sys.exit(0)
        except GLib.Error:
            continue
    elif name == " ".join(names) and (
        (verb == "state" and role != "label")
        or (verb == "frame-state" and role == "frame")
    ):
        print(" ".join(v.value_nick for v in node.get_state_set().get_states()))
        sys.exit(0)
    elif (
        verb == "click"
        and name == " ".join(names)
        and role
        in (
            "menu item",
            "list item",
            "push button",
            "button",
            "toggle button",
            "toggle",
            "check box",
        )
    ):
        for i in range(node.get_n_actions()):
            if node.get_action_name(i) in ("click", "activate", "press"):
                if node.do_action(i):
                    sys.exit(0)
if verb != "list":
    sys.exit(1)
