"""Test actual seat clicks, focus, quick settings and workspace transitions in a headless joe guest."""

import argparse
import json
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


def qualify(run, gpui_screenshots=None):
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
    for name in ["menu_fixture.py", "quick_settings_fixture.py", "shell_widgets.py"]:
        shutil.copy(here / name, g.run / "share" / name)
    g.shell(
        "mkdir -p /tmp/menu-test; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt; cp /mnt/menu_fixture.py /mnt/quick_settings_fixture.py /mnt/shell_widgets.py /tmp/menu-test; umount /mnt"
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
        if gpui_screenshots:
            # AccessKit supplies control bounds but no layer-window frame extents.
            x, y, w, h = map(int, widget("bounds", name).splitlines()[-1].split())
            return (x + w / 2) / 1920, (1080 - 44 + y + h / 2) / 1080
        x, bottom = map(int, widget("bar-point", name).split())
        return x / 1920, 1 - bottom / 1080

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

    if gpui_screenshots:
        wait(
            "GPUI preview login", lambda: g.shell("pgrep -x -u joe mundane-shell || :")
        )
        joe(
            "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true"
        )
        gpui_screenshots.mkdir(parents=True, exist_ok=True)
    else:
        print(joe("python3 /tmp/menu-test/quick_settings_fixture.py"), flush=True)
        g.shell("modprobe mac80211_hwsim")
        module = joe("pactl load-module module-null-sink sink_name=mundane_menu_check")
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
        tray_action = (
            "menu item Open folder" if gpui_screenshots else "button Open folder"
        )
        for anchor, content in [
            ("Start", "frame Start menu"),
            (
                "System menu",
                "frame System menu" if gpui_screenshots else "frame Quick Settings",
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
        if gpui_screenshots:
            key("meta_l", "s")
            wait("keyboard route to tray", lambda: "menu item Review tray" in rows())
            key("home")
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
                    events=[
                        dict(type="btn", data=dict(button="wheel-down", down=down))
                    ],
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
            wait(
                "running application for captures", lambda: "frame Calculator" in rows()
            )
            for anchor, content, name in [
                ("Review tray", tray_action, "tray"),
                ("Workspaces, current 1", "frame Workspaces", "workspaces"),
            ]:
                button(anchor)
                wait(name, lambda: content in rows())
                time.sleep(0.2)
                with (gpui_screenshots / (name + ".png")).open("wb") as out:
                    subprocess.run(
                        ["pnmtopng"], input=g.screen(name), stdout=out, check=True
                    )
                if name == "tray":
                    widget("click", "Recent documents ›")
                    wait("tray submenu", lambda: "menu item Week.txt" in rows())
                    widget("click", "Back")
                    wait("parent tray menu", lambda: tray_action in rows())
                key("esc")
            return
        button("System menu")
        wait(
            "wifi ready",
            lambda: any(
                s.startswith("toggle button Wi-Fi, ") and "Unavailable" not in s
                for s in rows()
            ),
        )
        was = joe("nmcli radio wifi")

        def radio():
            return "Wi-Fi, " + ("On" if joe("nmcli radio wifi") == "enabled" else "Off")

        widget("click", radio())
        wait("radio set", lambda: joe("nmcli radio wifi") != was)
        wait("radio label updated", lambda: "toggle button " + radio() in rows())
        widget("click", radio())
        wait("radio restored", lambda: joe("nmcli radio wifi") == was)
        prefs = "gsettings get org.gnome.desktop.notifications show-banners"

        def quiet():
            return "Do Not Disturb, " + ("Off" if joe(prefs) == "true" else "On")

        if joe(prefs) == "false":
            widget("click", quiet())
        widget("click", quiet())
        wait("quiet enabled", lambda: joe(prefs) == "false")
        joe(
            "gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications --method org.freedesktop.Notifications.Notify Review 0 '' 'Quiet test' 'Stored in history' '[]' '{}' 5000"
        )
        assert "label Quiet test" not in rows()
        history = next(
            s.removeprefix("button ")
            for s in rows()
            if s.startswith("button Notifications (")
        )
        widget("click", history)
        wait("quiet notification retained", lambda: "label Quiet test" in rows())
        widget("click", "Dismiss")
        widget("click", quiet())
        wait("banners restored", lambda: joe(prefs) == "true")
        before = joe("wpctl get-volume @DEFAULT_AUDIO_SINK@")
        for _ in range(20):
            if "focused" in widget("state", "Volume").split():
                break
            key("tab")
        assert "focused" in widget("state", "Volume").split()
        key("left" if "1.00" in before else "right")
        wait(
            "volume slider changed live sink",
            lambda: joe("wpctl get-volume @DEFAULT_AUDIO_SINK@") != before,
        )
        for name in ("Restart", "Shut Down"):
            widget("click", name)
            wait(
                "confirmation for " + name,
                lambda: "label " + name + " this computer?" in rows(),
            )
            widget("click", "Cancel")
            wait("confirmation cancelled", lambda: "button Cancel" not in rows())
            button("System menu")
            wait("quick settings reopened", lambda: "frame Quick Settings" in rows())
        key("esc")
        wait("quick settings dismissed", lambda: "frame Quick Settings" not in rows())
        print(
            "Wi-Fi, Do Not Disturb with retained history, volume and both power confirmations passed",
            flush=True,
        )
        button("Workspaces, current 1")
        wait("desk tiles", lambda: "toggle button Desk 2" in rows())
        assert "pressed" in widget("state", "Desk 1") or "checked" in widget(
            "state", "Desk 1"
        )
        assert "label Current · 1" in rows() and "label Empty" in rows()
        widget("click", "Desk 2")
        wait("tile switched", lambda: "button Workspaces, current 2" in rows())
        g.move(*point("Workspaces, current 2"))
        for down in (True, False):
            g.qmp(
                "input-send-event",
                events=[dict(type="btn", data=dict(button="wheel-down", down=down))],
            )
        wait("wheel switched", lambda: "button Workspaces, current 3" in rows())
        key("meta_l", "1")
        wait("keyboard switched", lambda: "button Workspaces, current 1" in rows())

        transitions()
        output = json.loads(joe("/usr/local/bin/mundane-de output list"))[0]
        mode = output["current"]
        command = shlex.join(
            [
                "/usr/local/bin/mundane-de",
                "output",
                "test",
                output["name"],
                str(mode["width"]),
                str(mode["height"]),
                str(round(mode["refresh_hz"] * 1000)),
                "2",
            ]
        )
        contrast_key = "org.gnome.desktop.a11y.interface high-contrast"
        previous = joe("gsettings get " + contrast_key)
        joe("gsettings set " + contrast_key + " true")
        joe(command)
        try:
            wait(
                "smaller logical monitor",
                lambda: (
                    json.loads(joe("/usr/local/bin/mundane-de output list"))[0]["scale"]
                    == 2
                ),
            )
            key("meta_l", "s")
            wait("scaled Quick Settings", lambda: "button Shut Down" in rows())
            width, height = map(int, widget("size", "Quick Settings").split())
            assert 0 < width <= 960 and 0 < height <= 540 - 48, (width, height)
            key("tab")
            pixels = g.screen("quick-settings-high-contrast").split(b"\n", 3)[3]
            assert pixels.count(bytes.fromhex("ffff00")) > 100
            for command_name in ("Lock", "Log Out", "Restart", "Shut Down"):
                x, y, w, h = map(int, widget("bounds", command_name).split())
                assert 0 <= x < x + w <= width and 0 <= y < y + h <= height, (
                    command_name,
                    x,
                    y,
                    w,
                    h,
                    width,
                    height,
                )
        finally:
            key("esc")
            joe("/usr/local/bin/mundane-de output revert")
            joe("gsettings set " + contrast_key + " " + previous)
        print(
            "High contrast, keyboard focus and panel bounds on a 960×540 logical display passed",
            flush=True,
        )
    finally:
        g.shell(f"kill {proc} 2>/dev/null || :")
        if not gpui_screenshots:
            joe("pactl unload-module " + module)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument(
        "--gpui-screenshots", type=Path, help="test the GPUI step 1 menus"
    )
    args = parser.parse_args()
    qualify(args.run, args.gpui_screenshots)
