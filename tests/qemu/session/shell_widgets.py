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
    elif name == " ".join(names) and verb == "focus":
        if node.grab_focus():
            sys.exit(0)
    elif name == " ".join(names) and verb == "state":
        print(" ".join(v.value_nick for v in node.get_state_set().get_states()))
        sys.exit(0)
    elif name == " ".join(names) and role in ("push button", "button", "toggle button"):
        for i in range(node.get_n_actions()):
            if node.get_action_name(i) in ("click", "activate", "press"):
                if node.do_action(i):
                    sys.exit(0)
if verb != "list":
    sys.exit(1)
