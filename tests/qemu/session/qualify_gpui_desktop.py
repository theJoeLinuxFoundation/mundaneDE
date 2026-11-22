"""Exercise desktop icons and real Wayland input contexts on an enforcing, headless joe guest."""

import argparse
import re
import shlex
import shutil
import time
from pathlib import Path

from qualify_gpui_outputs import plug
from qualify_gpui_settings import HERE, RT, Session
from qualify_menus import wait


def captions(s, folder):
    name = "Zebra field observations from the western foothills.txt"
    path = f"{folder}/{shlex.quote(name)}"
    s.joe(f"test ! -e {path}")
    try:
        s.joe(f"touch {path}")
        wait("long filename", lambda: "button " + name in s.tree())

        def bounds():
            return tuple(map(int, s.widget("bounds", name, "Desktop icons").split()))

        s.g.move(0.35, 0.45)
        x, y, w, h = bounds()
        # Count separate runs of text ink below the 32-pixel artwork in the tile.
        pixels = s.g.screen("caption-lines").split(b"\n", 3)[3]
        ink = []
        for row in range(y + h // 2, y + h):
            colors = [
                pixels[(row * 1920 + col) * 3 : (row * 1920 + col) * 3 + 3]
                for col in range(x + 8, x + w - 8)
            ]
            bg = max(set(colors), key=colors.count)
            if colors.count(bg) >= len(colors) // 2 and any(
                sum(abs(a - b) for a, b in zip(c, bg)) > 145 for c in colors
            ):
                ink.append(row)
        assert len([v for v in ink if v - 1 not in ink]) >= 2, (
            "two visible filename lines",
            ink,
        )
        s.shot("icon-label")
        s.g.move((x + w / 2) / 1920, (y + h / 2) / 1080)
        wait("hover expands the full wrapped caption", lambda: bounds()[3] > h + 10)
        s.shot("icon-label-hover")
        s.g.move(0.40, 0.40)
        wait("pointer exit restores the grid", lambda: bounds()[3] == h)
        s.key("meta_l", "d")
        s.key("end")
        wait(
            "selection expands without the pointer",
            lambda: (
                "focused" in s.widget("state", name, "Desktop icons").split()
                and bounds()[3] > h + 10
            ),
        )
        s.shot("icon-label-selected")
        s.key("home")
        wait("changing selection restores two lines", lambda: bounds()[3] == h)
        s.key("meta_l", "d")
    finally:
        s.joe(f"rm -f {path}")


def desktop(s):
    folder = shlex.quote(s.joe("xdg-user-dir DESKTOP"))
    work = s.joe("mktemp -d")
    item = "Desktop gate.txt"
    uri = f"{folder}/'Desktop gate.txt'"
    renamed = f"{folder}/'Rename gate.txt'"
    previous = s.joe("gio mime text/plain").splitlines()[0].split()[-1]
    schemes = (
        "org.gnome.desktop.interface color-scheme",
        "org.gnome.desktop.a11y.interface high-contrast",
    )
    old = [s.joe("gsettings get " + k) for k in schemes]

    def focused(name):
        return "focused" in s.widget("state", name, "Desktop icons").split()

    s.joe(f"test ! -e {uri} && test ! -e {renamed}")
    try:
        s.joe(f"test ! -e {uri} && test ! -e {renamed} && printf 'Desktop gate' >{uri}")
        wait("directory watch adds the file", lambda: "button " + item in s.tree())
        icons = s.widget("list", frame="Desktop icons").splitlines()
        assert (
            icons.index("button Home")
            < icons.index("button Trash")
            < icons.index("button " + item)
        )
        captions(s, folder)
        for label, scheme, contrast in (
            ("light", "prefer-light", "false"),
            ("dark", "prefer-dark", "false"),
            ("contrast", "prefer-dark", "true"),
        ):
            s.joe(
                f"gsettings set {schemes[0]} {scheme} && gsettings set {schemes[1]} {contrast}"
            )
            time.sleep(0.4)
            s.shot("desktop-" + label)
        s.joe(f"mv {uri} {renamed}")
        wait(
            "rename replaces previous button",
            lambda: (
                "button Rename gate.txt" in s.tree()
                and "button " + item not in s.tree()
            ),
        )
        s.joe(f"mv {renamed} {uri}")
        plug(s.g, True)
        wait(
            "two desktop layers",
            lambda: (
                s.tree().count("frame Desktop icons") == 2
                and s.tree().count("button " + item) == 2
            ),
        )
        plug(s.g, False)
        wait("one desktop layer", lambda: s.tree().count("frame Desktop icons") == 1)
        s.joe("gio mime text/plain org.gnome.TextEditor.desktop")
        # A process with a regular toplevel makes both halves of Super+D observable.
        s.launch(f"gnome-text-editor --standalone {work}/editor.txt", "desktop-editor")
        wait("editor mapped", lambda: any("editor.txt" in row for row in s.tree()))
        s.g.move(0.25, 0.3)
        s.key("meta_l", "d")
        wait("Super+D focuses Home", lambda: focused("Home"))
        s.key("down")
        assert focused("Trash")
        s.key("meta_l", "d")
        wait("previous window regains focus", lambda: not focused("Trash"))
        s.key("meta_l", "d")
        wait("desktop focused again", lambda: focused("Home"))
        s.key("down")
        s.key("down")
        assert focused(item)
        s.key("ret")
        wait(
            "default text handler opens selected file",
            lambda: any(item in r and ("frame" in r or "tab" in r) for r in s.tree()),
        )
        s.joe(f"rm {uri}")
        wait(
            "removed file disappears",
            lambda: (
                "button " + item
                not in s.widget("list", frame="Desktop icons").splitlines()
            ),
        )
        assert "org.gnome.Nautilus.desktop" in s.joe("gio mime inode/directory")
        s.widget("click", "Home", "Desktop icons")
        wait(
            "folder handler opened Home",
            lambda: any("frame Home" in r for r in s.tree()),
        )
        s.widget("click", "Trash", "Desktop icons")
        wait(
            "trash URI reaches Files", lambda: any("frame Trash" in r for r in s.tree())
        )
        print(
            "Home, Trash, live add/rename/remove, output lifetimes, keyboard selection, Super+D and default handlers passed.",
            flush=True,
        )
    finally:
        plug(s.g, False)
        for key, value in zip(schemes, old):
            s.joe(f"gsettings set {key} {value}")
        if previous.endswith(".desktop"):
            s.joe("gio mime text/plain " + shlex.quote(previous))
        s.joe(f"rm -f {uri} {renamed}; nautilus --quit >/dev/null 2>&1 || :")


def input_method(s):
    for path in (HERE.parents[1] / "nested/engine.py", HERE / "ime_entries.py"):
        shutil.copyfile(path, s.g.run / "share" / path.name)
    s.g.shell(
        "mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && cp /mnt/engine.py /mnt/ime_entries.py /tmp && umount /mnt"
    )
    previous = s.joe("ibus engine || :")
    work = s.joe("mktemp -d")
    engine = s.launch("python3 /tmp/engine.py", "conversion-engine")
    clients = [engine]

    def logged(app, pattern):
        return bool(
            s.g.shell(
                "grep -F -- "
                + shlex.quote(pattern)
                + f" /tmp/mundane-{app}.log | tail -1"
            )
        )

    def candidates():
        return "frame Input method candidates" in s.tree()

    def open_candidates():
        before = s.g.screen("before-ime").split(b"\n", 3)[3]
        s.key("j")
        wait(
            "GPUI candidate page",
            lambda: "button 日本" in s.tree() and "button 二" in s.tree(),
        )
        return before

    def placement(before):
        log = s.g.shell(
            "grep -E 'set_(cursor_rectangle|window_geometry)' /tmp/mundane-input.log | tail -45"
        )
        gx, gy, width, height = map(
            int,
            re.findall(r"set_window_geometry\((-?\d+), (-?\d+), (\d+), (\d+)\)", log)[
                -1
            ],
        )
        origin = s.g.shell(
            f"journalctl -b -o cat --no-pager -g 'mapped window {width}x{height}' -n1"
        )
        ox, oy = map(
            int, re.search(r"springing in to \((-?\d+), (-?\d+)\)", origin).groups()
        )
        x, y, _, height = map(
            int,
            re.findall(r"set_cursor_rectangle\((-?\d+), (-?\d+), (\d+), (\d+)\)", log)[
                -1
            ],
        )
        x += ox - gx
        y += oy - gy + height
        pixels = s.g.screen("after-ime").split(b"\n", 3)[3]
        changes = 0
        for row in range(y + 5, y + 55):
            off = 3 * (1920 * row + x + 4)
            changes += sum(
                a != b for a, b in zip(before[off : off + 180], pixels[off : off + 180])
            )
        assert changes > 900, (x, y, changes)
        return x, y

    try:
        wait(
            "IBus engine registered",
            lambda: logged("conversion-engine", "engine registered"),
        )
        s.joe("python3 /tmp/engine.py --select")
        clients.append(
            s.launch(
                "env GTK_IM_MODULE=wayland WAYLAND_DEBUG=client python3 /tmp/ime_entries.py",
                "input",
            )
        )
        wait("first context enabled", lambda: logged("input", ".enable()"))
        time.sleep(0.4)
        first = placement(open_candidates())
        assert logged("input", 'preedit_string("に"')
        s.shot("input-method")
        s.key("k")
        wait(
            "keyboard commit in first GTK entry",
            lambda: s.joe(f"cat '{RT}/First input'") == "日本" and not candidates(),
        )
        s.key("tab")
        time.sleep(0.4)
        second = placement(open_candidates())
        assert second[1] > first[1] + 80
        s.widget("click", "二", "Input method candidates")
        wait(
            "candidate click commits to the second GTK entry",
            lambda: s.joe(f"cat '{RT}/Second input' || :") == "二" and not candidates(),
        )
        open_candidates()
        s.joe(f"mkdir {work}/profile")
        prefs = 'user_pref("widget.wayland.text_input_v3.enabled",true);\nuser_pref("browser.aboutwelcome.enabled",false);\nuser_pref("browser.shell.checkDefaultBrowser",false);\nuser_pref("browser.startup.homepage_override.mstone","ignore");'
        page = '<meta charset="utf-8"><title>Conversion test</title><input aria-label="Conversion field" style="width:240px;height:40px;background:#aa33bb" oninput="document.title=this.value">'
        s.joe(
            f"printf %s {shlex.quote(prefs)} >{work}/profile/user.js && printf %s {shlex.quote(page)} >{work}/input.html"
        )
        clients.append(
            s.launch(
                f"env GTK_IM_MODULE=wayland MOZ_ENABLE_WAYLAND=1 WAYLAND_DEBUG=client firefox --no-remote --profile {work}/profile file://{work}/input.html",
                "ime-firefox",
            )
        )
        # Firefox initially enables text-input on its browser chrome. Click the
        # form itself on scanout before checking where the old preedit went.
        fill = bytes.fromhex("aa33bb") * 20

        def form_point():
            pixels = s.g.screen("conversion-field").split(b"\n", 3)[3]
            start = pixels.find(fill)
            if start >= 0:
                y, x = divmod(start // 3, 1920)
                return ((x + 10) / 1920, (y + 10) / 1080)

        wait("Firefox presents the conversion field", form_point, 35)
        s.g.click(*form_point())
        wait("Firefox focuses the form", lambda: logged("ime-firefox", ".enable()"))
        wait(
            "focus change clears old candidates",
            lambda: not candidates() and logged("input", ".leave(wl_surface"),
        )
        s.key("k")
        wait(
            "previous preedit cannot commit in Firefox",
            lambda: logged("ime-firefox", 'set_title("k '),
        )
        s.key("ctrl", "a")
        s.key("backspace")
        open_candidates()
        s.key("k")
        wait(
            "Firefox form receives real composition",
            lambda: logged("ime-firefox", 'set_title("日本') and not candidates(),
        )
        print(
            "IBus inline preedit, GTK keyboard and pointer commits, two measured caret positions and Firefox focus transfer and conversion passed.",
            flush=True,
        )
    finally:
        if previous and previous != "mundane-test":
            s.joe("ibus engine " + shlex.quote(previous) + " || :")
        s.g.shell("kill " + " ".join(map(str, clients)) + " 2>/dev/null || :")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("screenshots", type=Path)
    a = p.parse_args()
    s = Session(a.run, a.screenshots)
    desktop(s)
    input_method(s)
