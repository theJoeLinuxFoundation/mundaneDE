"""Test actual seat clicks, focus, quick settings and workspace transitions in a headless joe guest."""

import argparse
from pathlib import Path
import shlex
import shutil
import subprocess
import time
from guest import Guest


def wait(name, check, seconds=15):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if check():
            return
        time.sleep(0.12)
    raise AssertionError(name)


def qualify(run, screenshots):
    g = Guest(run)
    assert g.qmp("query-display-options")["type"] == "none"
    wait(
        "1920×1080 scanout",
        lambda: g.screen("menu-scanout").startswith(b"P6\n1920 1080\n"),
    )
    assert all(
        n["ro"]
        for n in g.qmp("query-named-block-nodes")
        if n["node-name"] in ("base", "basefile")
    )
    here = Path(__file__).parent
    for name in ["menu_fixture.py", "shell_widgets.py"]:
        shutil.copy(here / name, g.run / "share" / name)
    g.shell(
        "mkdir -p /tmp/menu-test; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt; cp /mnt/menu_fixture.py /mnt/shell_widgets.py /tmp/menu-test; umount /mnt"
    )

    def joe(cmd):
        return g.shell(
            "su joe -s /bin/sh -c "
            + shlex.quote(
                "export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus WAYLAND_DISPLAY=wayland-1; "
                + cmd
            )
        )

    def widget(verb, name=""):
        return joe(
            "python3 -W ignore /tmp/menu-test/shell_widgets.py "
            + shlex.join([verb, name])
        )

    def rows():
        return widget("list").splitlines()

    def point(name):
        x, y, w, h = map(int, widget("bounds", name).splitlines()[-1].split())
        return (x + w / 2) / 1920, (1080 - 44 + y + h / 2) / 1080

    def button(name):
        g.click(*point(name))

    def key(*keys):
        g.key(*keys)
        time.sleep(0.15)

    def transitions():
        # The coloured probe covers the centre of workspace 1. Workspace 4 is
        # empty. Sample actual scanout through the switch and compare both modes.
        def pixel():
            pixels = g.screen("workspace-transition").split(b"\n", 3)[3]
            i = (540 * 1920 + 960) * 3
            return pixels[i : i + 3]

        for animate in (True, False):
            joe(
                "gsettings set org.gnome.desktop.interface enable-animations "
                + str(animate).lower()
            )
            time.sleep(0.25)
            first = pixel()
            g.key("meta_l", "4")
            values = [pixel() for _ in range(16)]
            time.sleep(0.35)
            last = pixel()
            assert first != last
            intermediate = set(values) - {first, last}
            assert bool(intermediate) == animate, (animate, intermediate)
            key("meta_l", "1")
            time.sleep(0.4)
        joe("gsettings set org.gnome.desktop.interface enable-animations true")
        print(
            "Workspace tiles, scrolling, keyboard switching, intermediate scanout frames and reduced motion passed",
            flush=True,
        )

    wait("mundaneDE login", lambda: g.shell("pgrep -x -u joe mundane-shell || :"))
    joe(
        "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true"
    )
    screenshots.mkdir(parents=True, exist_ok=True)
    proc = int(
        joe(
            "python3 /tmp/menu-test/menu_fixture.py </dev/null >/tmp/tray-menu.log 2>&1 & echo $!"
        )
    )
    try:
        key("esc")
        key("meta_l", "1")
        wait("first workspace", lambda: "button Workspaces, current 1" in rows())
        joe(
            "busctl --user call org.joelinux.MundaneSessionTest /org/joelinux/MundaneSessionTest org.freedesktop.Application Activate 'a{sv}' 0"
        )
        wait(
            "probe ready",
            lambda: (
                "ready " in g.shell("cat /run/user/1000/mundane-login-test.log || :")
            ),
        )
        log = lambda: g.shell("cat /run/user/1000/mundane-login-test.log")
        # This point must really be over the client, so its failure to receive
        # the dismissal click establishes that the compositor consumed it.
        wait(
            "current probe mapped and focused",
            lambda: (
                "frame mundane session probe" in rows()
                and "active" in widget("frame-state", "mundane session probe").split()
            ),
        )
        clicks = log().splitlines().count("click")
        g.move(
            0.37, 0.38
        )  # Deliver fresh absolute axes after GDM reopens the virtual seat.
        g.click(0.5, 0.5)
        wait(
            "underlying app receives ordinary clicks",
            lambda: log().splitlines().count("click") == clicks + 1,
        )
        tray_action = "menu item Open folder"
        for anchor, content in [
            ("Start", "frame Start menu"),
            (
                "System menu",
                "frame Quick Settings",
            ),
            ("Workspaces, current 1", "frame Workspaces"),
            ("Review tray", tray_action),
        ]:

            def open_menu():
                button(anchor)
                wait("open " + anchor, lambda: content in rows())

            def closed():
                wait("dismiss " + anchor, lambda: content not in rows())

            def check_focus():
                before = log().splitlines().count("key a")
                key("a")
                wait(
                    "focus restored after " + anchor,
                    lambda: log().splitlines().count("key a") == before + 1,
                )

            open_menu()
            button(anchor)
            closed()
            check_focus()
            open_menu()
            key("esc")
            closed()
            check_focus()
            open_menu()
            clicks = log().splitlines().count("click")
            g.click(0.5, 0.5)
            closed()
            assert log().splitlines().count("click") == clicks, anchor
            check_focus()
            print(
                anchor,
                "repeat click, Escape, consumed outside click and restored focus passed",
                flush=True,
            )
        joe("rm -f /run/user/1000/tray-menu-action")
        button("Review tray")
        wait("tray action", lambda: tray_action in rows())
        widget("click", "Open folder")
        wait(
            "tray owner received action",
            lambda: joe("test -f /run/user/1000/tray-menu-action && echo yes || :"),
        )
        key("meta_l", "s")
        wait("keyboard route to tray", lambda: "frame Quick Settings" in rows())
        for _ in range(16):
            if (
                "focused"
                in joe(
                    "python3 -W ignore /tmp/menu-test/shell_widgets.py --frame 'Quick Settings' state 'Review tray'"
                ).split()
            ):
                break
            key("tab")
        assert (
            "focused"
            in joe(
                "python3 -W ignore /tmp/menu-test/shell_widgets.py --frame 'Quick Settings' state 'Review tray'"
            ).split()
        )
        key("ret")
        wait("tray menu from keyboard", lambda: tray_action in rows())
        key("esc")
        button("Workspaces, current 1")
        widget("click", "Desk 2")
        wait("desk selected", lambda: "button Workspaces, current 2" in rows())
        g.move(*point("Workspaces, current 2"))
        for down in (True, False):
            g.qmp(
                "input-send-event",
                events=[dict(type="btn", data=dict(button="wheel-down", down=down))],
            )
        wait("wheel switched", lambda: "button Workspaces, current 3" in rows())
        key("meta_l", "1")
        wait("first desk again", lambda: "button Workspaces, current 1" in rows())
        button("Workspaces, current 1")
        for letter in "probe":
            key(letter)
        key("ret")
        wait(
            "finder selected the probe",
            lambda: "active" in widget("frame-state", "mundane session probe"),
        )
        transitions()
        button("Workspaces, current 1")
        widget("click", "Move →")
        button("Workspaces, current 1")
        for letter in "probe":
            key(letter)
        key("ret")
        wait(
            "finder restores the window on its destination desk",
            lambda: (
                "button Workspaces, current 2" in rows()
                and "active" in widget("frame-state", "mundane session probe")
            ),
        )
        key("meta_l", "q")
        key("meta_l", "1")
        widget("click", "Calculator")
        wait("running application for captures", lambda: "frame Calculator" in rows())
        for anchor, content, name in [
            ("Review tray", tray_action, "tray"),
            ("Workspaces, current 1", "frame Workspaces", "workspaces"),
        ]:
            button(anchor)
            wait(name, lambda: content in rows())
            time.sleep(0.2)
            with (screenshots / (name + ".png")).open("wb") as out:
                subprocess.run(
                    ["pnmtopng"], input=g.screen(name), stdout=out, check=True
                )
            if name == "tray":
                widget("click", "Recent documents ›")
                wait("tray submenu", lambda: "menu item Week.txt" in rows())
                widget("click", "Back")
                wait("parent tray menu", lambda: tray_action in rows())
            key("esc")
    finally:
        g.shell(f"kill {proc} 2>/dev/null || :")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--screenshots", type=Path, help="directory for menu scanouts")
    args = parser.parse_args()
    qualify(args.run, args.screenshots or args.run / "screenshots")
