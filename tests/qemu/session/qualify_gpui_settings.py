"""Check the GPUI launcher and Quick Settings using an isolated, enforcing joe guest."""

import argparse
import json
import shlex
import shutil
import subprocess
import time
from pathlib import Path

from accessibility import suite_checks
from guest import Guest
from qualify_menus import wait

HERE = Path(__file__).parent
RT = "/run/user/1000"
FIX = "/tmp/mundane-gpui-step2"


class Session:
    def __init__(self, run, output):
        self.g = Guest(run)
        g = self.g
        assert g.qmp("query-display-options")["type"] == "none"
        nodes = {n["node-name"]: n["ro"] for n in g.qmp("query-named-block-nodes")}
        assert nodes["base"] and nodes["basefile"] and not nodes["layer"]
        assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
        self.output = output
        output.mkdir(parents=True, exist_ok=True)
        for path in [
            HERE / "device_fixture.py",
            HERE / "brightness_key.py",
            HERE / "shell_widgets.py",
            *(HERE / "accessibility").glob("*.py"),
        ]:
            shutil.copyfile(path, g.run / "share" / path.name)
        g.shell(
            f"install -d -m 755 {FIX}; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && cp /mnt/*.py {FIX}/ && umount /mnt"
        )
        self.ready()

    def ready(self):
        wait(
            "GPUI login", lambda: self.g.shell("pgrep -x -u joe mundane-shell || :"), 45
        )
        self.joe(
            "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true"
        )
        wait("presented taskbar", lambda: "button Start" in self.tree())

    def joe(self, cmd):
        return self.g.shell(
            "su joe -s /bin/sh -c "
            + shlex.quote(
                f"export PATH=/usr/local/bin:$PATH XDG_RUNTIME_DIR={RT} DBUS_SESSION_BUS_ADDRESS=unix:path={RT}/bus "
                "WAYLAND_DISPLAY=wayland-1 XDG_CURRENT_DESKTOP=mundane XDG_SESSION_TYPE=wayland; "
                + cmd
            )
        )

    def key(self, *keys):
        self.g.key(*keys)
        time.sleep(0.15)

    def widget(self, verb, label="", frame=None):
        args = (["--frame", frame] if frame else []) + [verb, label]
        return self.joe(f"python3 -W ignore {FIX}/shell_widgets.py " + shlex.join(args))

    def tree(self):
        return self.widget("list").splitlines()

    def click(self, name):
        self.widget("click", name)

    def launch(self, cmd, name):
        return int(
            self.joe(f"{cmd} </dev/null >/tmp/mundane-{name}.log 2>&1 & echo $!")
        )

    def read(self, name):
        return self.g.shell(f"cat {RT}/{name} 2>/dev/null || :")

    def shot(self, name):
        time.sleep(0.25)
        data = self.g.screen(name)
        assert data.startswith(b"P6\n1920 1080\n255\n")
        with (self.output / (name + ".png")).open("wb") as f:
            subprocess.run(["pnmtopng"], input=data, stdout=f, check=True)

    def pref(self, key, value):
        self.joe(f"gsettings set org.gnome.desktop.{key} {value}")


def launcher(s):
    s.key("esc")
    s.key("meta_l", "spc")
    wait("launcher", lambda: "label Pinned" in s.tree())
    s.shot("after-start")
    rows = s.widget("list", frame="Start menu").splitlines()
    assert (
        rows.index("label Pinned")
        < rows.index("button Files")
        < rows.index("label All applications")
    )
    assert "label Start menu" not in rows and not any("button Pin " in r for r in rows)
    x, y, w, h = map(int, s.widget("bounds", "Files", "Start menu").split())
    # The native overlay sits six logical pixels above the 44-pixel bar.
    _, height = map(int, s.widget("size", "Start menu").split())
    pos = ((8 + x + w / 2) / 1920, (1080 - 44 - 6 - height + y + h / 2) / 1080)
    # QEMU omits unchanged axes after a new login reopens the tablet.
    s.g.move(0.48, 0.42)
    time.sleep(0.1)
    for button in ("right", "left"):
        s.g.move(*pos)
        time.sleep(0.1)  # Let the emulated tablet report the new target.
        for down in (True, False):
            s.g.qmp(
                "input-send-event",
                events=[{"type": "btn", "data": {"button": button, "down": down}}],
            )
            if down:
                time.sleep(0.7 if button == "left" else 0.1)
        wait(button + " pin menu", lambda: "menu item Unpin Files" in s.tree())
        assert "frame Files" not in s.tree()
        if button == "right":
            s.shot("application-actions")
        s.key("esc")
        wait(
            "focus returned to tile",
            lambda: "focused" in s.widget("state", "Files", "Start menu").split(),
        )
    s.key("menu")
    wait("Menu key", lambda: "menu item Unpin Files" in s.tree())
    s.key("ret")
    wait(
        "pin removed",
        lambda: s.widget("description", "Files", "Start menu") == "Application",
    )
    s.key("esc")
    s.key("meta_l", "spc")
    for letter in "arithmetic":
        s.key(letter)
    wait(
        "flat search result",
        lambda: (
            "button Calculator" in s.widget("list", frame="Start menu").splitlines()
        ),
    )
    s.widget("click", "Calculator", "Start menu")
    wait(
        "flat row activated through AT-SPI",
        lambda: "frame Calculator" in s.tree() and "frame Start menu" not in s.tree(),
    )
    s.key("meta_l", "q")
    s.joe("gsettings reset org.joelinux.mundane.shell favorite-apps")
    print(
        "Pinned grid, full list, right click, long press, Menu key and focus restoration passed.",
        flush=True,
    )


def controls(s):
    s.key("esc")
    s.g.shell("modprobe mac80211_hwsim")
    module = s.joe("pactl load-module module-null-sink sink_name=gpui_quick_test")
    try:
        s.key("meta_l", "s")
        wait("Wi-Fi tile", lambda: s.widget("description", "Wi-Fi") in ("On", "Off"))
        s.shot("quick-settings")
        # A named button in the tree can still have invisible symbolic artwork.
        # Sample the middle of each target, clear of its focus ring and tooltip.
        frame = s.g.screen("power-icon-ink").split(b"\n", 3)[3]
        width, height = map(int, s.widget("size", "Quick Settings").split())
        for label in ("Lock", "Log Out", "Suspend", "Restart", "Shut Down"):
            x, y, w, h = map(int, s.widget("bounds", label, "Quick Settings").split())
            rx = 1920 - 8 - width + x + w // 2
            ry = 1080 - 44 - 6 - height + y + h // 2
            pixels = [
                frame[3 * (row * 1920 + col) : 3 * (row * 1920 + col + 1)]
                for row in range(ry - 12, ry + 12)
                for col in range(rx - 12, rx + 12)
            ]
            assert (
                max(pixels.count(color) for color in set(pixels)) < len(pixels) - 16
            ), label
        original = s.joe("nmcli radio wifi")
        for expected in [
            ("disabled" if original == "enabled" else "enabled"),
            original,
        ]:
            s.click("Wi-Fi")
            wait(
                "NetworkManager radio " + expected,
                lambda expected=expected: s.joe("nmcli radio wifi") == expected,
            )
            wait(
                "tile state",
                lambda expected=expected: (
                    ("pressed" in s.widget("state", "Wi-Fi").split())
                    == (expected == "enabled")
                ),
            )
        pref = "org.gnome.desktop.notifications show-banners"
        previous = s.joe("gsettings get " + pref)
        for expected in [("false" if previous == "true" else "true"), previous]:
            s.click("Do Not Disturb")
            wait(
                "DND",
                lambda expected=expected: s.joe("gsettings get " + pref) == expected,
            )
            wait(
                "DND tile",
                lambda expected=expected: (
                    ("pressed" in s.widget("state", "Do Not Disturb").split())
                    == (expected == "false")
                ),
            )
        s.widget("focus", "Volume")
        volume = s.joe("wpctl get-volume @DEFAULT_AUDIO_SINK@")
        s.key("left" if "1.00" in volume else "right")
        wait(
            "PipeWire sink level",
            lambda: s.joe("wpctl get-volume @DEFAULT_AUDIO_SINK@") != volume,
        )
        for action in ("Restart", "Shut Down"):
            s.click(action)
            wait(
                "Cancel initially focused",
                lambda: "focused" in s.widget("state", "Cancel").split(),
            )
            s.key("tab")
            assert "focused" in s.widget("state", action).split()
            s.key("esc")
            wait("cancelled power operation", lambda: "button Cancel" not in s.tree())
            assert "frame Quick Settings" in s.tree()
        s.key("esc")
        output = json.loads(s.joe("mundane-de output list"))[0]
        mode = output["current"]
        s.pref("a11y.interface high-contrast", "true")
        s.joe(
            shlex.join(
                [
                    "mundane-de",
                    "output",
                    "test",
                    output["name"],
                    str(mode["width"]),
                    str(mode["height"]),
                    str(round(mode["refresh_hz"] * 1000)),
                    "2",
                ]
            )
        )
        try:
            s.key("meta_l", "s")
            wait("scaled panel", lambda: "button Shut Down" in s.tree())
            width, height = map(int, s.widget("size", "Quick Settings").split())
            # The GPUI AccessKit bridge reports WINDOW bounds in device pixels.
            assert 0 < width / 2 <= 960 and 0 < height / 2 <= 540 - 44, (width, height)
            s.shot("scaled-quick-settings")
            for name in ("Lock", "Log Out", "Restart", "Shut Down"):
                x, y, w, h = map(int, s.widget("bounds", name).split())
                assert 0 <= x < x + w <= width and 0 <= y < y + h <= height, (
                    name,
                    x,
                    y,
                    w,
                    h,
                )
        finally:
            s.key("esc")
            s.joe("mundane-de output revert")
            s.pref("a11y.interface high-contrast", "false")
    finally:
        s.joe("pactl unload-module " + module)
    print(
        "Virtual Wi-Fi, live volume, Do Not Disturb, power cancellation and scaled panel bounds passed.",
        flush=True,
    )


def laptop(s):
    # The shell sees a private system bus for the virtual laptop. Everything else
    # in this guest, including its GDM login, retains its original system bus.
    g = s.g
    binary = "/usr/local/bin/mundane-shell"
    g.shell(
        f"test ! -e {binary}.fixture-real && install -d -o joe -g users {FIX}/bus && mount -t tmpfs tmpfs /sys/class/backlight && mkdir /sys/class/backlight/virt-panel && echo 400 >/sys/class/backlight/virt-panel/max_brightness && echo 100 >/sys/class/backlight/virt-panel/brightness && chown joe /sys/class/backlight/virt-panel/brightness"
    )
    g.shell(f"rm -f {FIX}/bus/events")
    address = f"unix:path={FIX}/bus/system"
    daemon = int(s.joe(f"dbus-daemon --session --fork --address={address} --print-pid"))
    fixture = s.launch(
        f"DBUS_SYSTEM_BUS_ADDRESS={address} DBUS_SESSION_BUS_ADDRESS={address} python3 {FIX}/device_fixture.py {FIX}/bus/events /sys/class/backlight/virt-panel",
        "laptop",
    )
    try:
        wrapper = f'#!/bin/sh\nexport DBUS_SYSTEM_BUS_ADDRESS={address}\nexec {binary}.fixture-real "$@"\n'
        g.shell(
            f"mv {binary} {binary}.fixture-real && printf %s {shlex.quote(wrapper)} >{binary} && chmod 755 {binary} && pkill -x mundane-shell"
        )
        wait(
            "restarted shell",
            lambda: (
                "fixture-real"
                in g.shell(
                    "ps -eo args | grep '^/usr/local/bin/mundane-shell.fixture-real' || :"
                )
            ),
        )
        s.key("meta_l", "s")
        wait("virtual laptop", lambda: "slider Brightness" in s.tree())
        assert "status bar Battery charge" in s.tree()
        s.widget("focus", "Brightness")
        s.key("right")
        wait(
            "logind native backlight units",
            lambda: "brightness 104" in g.shell(f"cat {FIX}/bus/events || :"),
        )
        # A laptop media key changes the virtual backlight through logind.
        g.shell("modprobe uinput")
        # Pass this guest device only to the unprivileged key fixture.
        g.shell(
            f"su joe -s /bin/sh -c 'python3 {FIX}/brightness_key.py up' 3<>/dev/uinput"
        )
        wait("brightness key feedback", lambda: "frame Brightness" in s.tree())
        assert "brightness 124" in g.shell(f"cat {FIX}/bus/events")
        wait("brightness OSD expired", lambda: "frame Brightness" not in s.tree())
        s.click("Devices and connections")
        for action, event in [("Cancel pairing", "cancelled"), ("Confirm", "paired")]:
            wait(
                "available Bluetooth device",
                lambda: "button Pair Guest keyboard" in s.tree(),
            )
            if action == "Confirm":
                s.widget("focus", "Devices and connections")
                for _ in range(12):
                    s.key("tab")
                    if "focused" in s.widget("state", "Pair Guest keyboard").split():
                        break
                assert "focused" in s.widget("state", "Pair Guest keyboard").split()
                _, y, _, h = map(int, s.widget("bounds", "Pair Guest keyboard").split())
                power_y = int(s.widget("bounds", "Lock", "Quick Settings").split()[1])
                assert 0 <= y < y + h <= power_y, (y, h, power_y)
                s.key("ret")
            else:
                # AT activation of a clipped device must dispatch to this identity.
                s.click("Pair Guest keyboard")
            wait("BlueZ confirmation", lambda: "button Cancel pairing" in s.tree())
            assert any("123456" in row for row in s.tree())
            s.click(action)
            wait(
                "BlueZ " + event,
                lambda event=event: (
                    event in g.shell(f"cat {FIX}/bus/events").splitlines()
                ),
            )
        g.shell(f"kill {fixture}")
        # NameOwnerChanged removes the departed devices from the open panel.
        wait("removed battery", lambda: "status bar Battery charge" not in s.tree())
        s.key("esc")
    except Exception:
        print("Laptop controls at failure:", s.tree(), flush=True)
        print(
            "Laptop service events:", g.shell(f"cat {FIX}/bus/events || :"), flush=True
        )
        raise
    finally:
        g.shell(
            f"if test -e {binary}.fixture-real; then mv -f {binary}.fixture-real {binary}; fi; systemctl stop gdm; kill {daemon} {fixture} 2>/dev/null || :; umount /sys/class/backlight; systemctl start gdm"
        )
        s.ready()
    print(
        "Virtual backlight, UPower battery, BlueZ confirm/cancel and removal passed.",
        flush=True,
    )


def forms(s):
    pid = s.launch("mundane-settings --page users", "forms")
    try:
        wait("Users page", lambda: "button Change password" in s.tree())
        s.click("Change password")
        wait("masked field", lambda: "password text Password" in s.tree())
        for key in ("g", "u", "e", "s", "t"):
            s.key(key)
        assert not any(
            "guest" in row.lower()
            for row in s.widget("list", frame="Change password").splitlines()
        )
        s.key("ret")
        wait("second masked field", lambda: "frame Confirm password" in s.tree())
        s.key("esc")
        wait(
            "password flow cancelled", lambda: "frame Confirm password" not in s.tree()
        )
        s.click("Software")
        wait("remotes page", lambda: "button Add remote" in s.tree())
        s.click("Add remote")
        wait("remote form", lambda: "frame Add Flatpak remote" in s.tree())
        fields = [
            r
            for r in s.widget("list", frame="Add Flatpak remote").splitlines()
            if r.startswith("entry ")
        ]
        assert len(fields) == 2, fields
        s.key("q")
        s.key("tab")
        assert (
            "focused" in s.widget("state", fields[1][6:], "Add Flatpak remote").split()
        )
        s.key("esc")
        wait(
            "remote form cancelled", lambda: "frame Add Flatpak remote" not in s.tree()
        )
        s.click("Compositor")
        wait("spring slider", lambda: "slider Damping" in s.tree())
        s.widget("focus", "Damping")
        s.key("end")
        wait(
            "saved spring",
            lambda: "damping = 60" in s.joe("cat ~/.config/mundane/config.toml || :"),
        )
        s.key("left")
        wait(
            "saved second step",
            lambda: "damping = 59" in s.joe("cat ~/.config/mundane/config.toml || :"),
        )
        print(
            "Masked entry, confirmation, two-field form, cancellation and saved slider values passed.",
            flush=True,
        )
    finally:
        s.g.shell(f"kill {pid} 2>/dev/null || :")


def appearance_and_speech(s, suite):
    g = s.g
    for source, name in [
        (suite / "mundane-terminal", "mundane-terminal"),
        (suite / "examples/motion", "suite-motion"),
    ]:
        subprocess.run(
            ["objcopy", "--strip-debug", source, g.run / "share" / name], check=True
        )
    g.shell(
        f"install -d /tmp/m7; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && install -m755 /mnt/mundane-terminal /usr/local/bin && install -m755 /mnt/suite-motion /tmp/m7 && umount /mnt; install -d -o joe -g users /home/joe/.local/share/orca; cp {FIX}/recording_speech.py {FIX}/orca-customizations.py /home/joe/.local/share/orca/"
    )
    s.pref("interface color-scheme", "prefer-light")
    s.pref("interface accent-color", "blue")
    s.pref("a11y.interface high-contrast", "false")
    s.pref("interface enable-animations", "true")
    reader = "org.gnome.desktop.a11y.applications screen-reader-enabled"
    previous_reader = s.joe("gsettings get " + reader)
    s.joe("gsettings set " + reader + " false")
    wait(
        "reader stopped",
        lambda: not g.shell("pgrep -u joe -f '^/usr/bin/python3 /usr/bin/orca' || :"),
    )
    s.joe("rm -f " + RT + "/mundane-speech.jsonl")
    s.joe("gsettings set " + reader + " true")
    native = None
    settings = None

    def spoken():
        return [
            json.loads(line)[1] for line in s.read("mundane-speech.jsonl").splitlines()
        ]

    def hear(offset, word):
        wait(
            "Orca says " + word,
            lambda: any(word.casefold() in t.casefold() for t in spoken()[offset:]),
        )

    try:
        wait("Orca speaking", lambda: spoken())
        native = s.launch(f"python3 {FIX}/style.py", "style")
        wait("native style client", lambda: "frame Native appearance test" in s.tree())
        suite_checks.drag_from_centre(g, 0.12, 0.35)
        offset = len(spoken())
        s.key("ctrl", "alt", "tab")
        hear(offset, "Start")
        hear(offset, "Taskbar")
        s.key("ret")
        hear(offset, "Search applications")
        s.key("esc")
        s.key("esc")
        offset = len(spoken())
        s.key("meta_l", "s")
        hear(offset, "Do Not Disturb")
        s.joe(
            "gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications --method org.freedesktop.Notifications.Notify Files 0 '' 'Archive extracted' 'Three documents' '[]' '{}' 1"
        )
        wait(
            "history control",
            lambda: any(
                row.startswith("button Notifications (")
                and row != "button Notifications (0)"
                for row in s.tree()
            ),
        )
        history = next(
            row.removeprefix("button ")
            for row in s.tree()
            if row.startswith("button Notifications (")
        )
        s.click(history)
        wait("notification history", lambda: "article Archive extracted" in s.tree())
        offset = len(spoken())
        s.key("down")
        hear(offset, "Archive extracted")
        s.key("esc")
        settings = s.launch("mundane-settings --page appearance", "settings")
        wait("appearance controls", lambda: "toggle button Dark appearance" in s.tree())
        # The output scale stays at one while Settings changes logical text size.
        start_width = int(s.widget("bounds", "Start", "Taskbar").split()[2])
        s.pref("interface text-scaling-factor", "1.25")
        wait(
            "scaled live taskbar",
            lambda: (
                int(s.widget("bounds", "Start", "Taskbar").split()[2]) > start_width
            ),
        )
        s.pref("interface text-scaling-factor", "1.0")
        wait(
            "default taskbar restored",
            lambda: (
                int(s.widget("bounds", "Start", "Taskbar").split()[2]) == start_width
            ),
        )
        s.click("Dark appearance")
        wait("native app dark", lambda: json.loads(s.read("gtk-style"))["dark"])
        s.widget("focus", "Dark appearance")
        offset = len(spoken())
        s.key("tab")
        hear(offset, "High contrast")
        assert "focused" in s.widget("state", "High contrast").split()
        s.key("spc")
        wait("GTK contrast", lambda: json.loads(s.read("gtk-style"))["contrast"])
        s.click("High contrast")
        s.click("Reduce motion")
        wait(
            "GTK reduced motion",
            lambda: not json.loads(s.read("gtk-style"))["animations"],
        )
        suite_checks.check(
            g,
            s.joe,
            s.launch,
            s.click,
            lambda: "\n".join(s.tree()),
            s.read,
            wait,
            gpui_shell=True,
        )
    finally:
        s.joe(f"gsettings set {reader} {previous_reader}")
        for pid in (settings, native):
            if pid:
                g.shell(f"kill {pid} 2>/dev/null || :")
    for variant, scheme, contrast in [
        ("dark", "prefer-dark", "false"),
        ("contrast", "prefer-dark", "true"),
        ("light", "prefer-light", "false"),
    ]:
        s.pref("interface color-scheme", scheme)
        s.pref("a11y.interface high-contrast", contrast)
        time.sleep(0.4)
        for key, page in [("spc", "start"), ("s", "quick")]:
            s.key("meta_l", key)
            s.shot(f"{variant}-{page}")
            s.key("esc")
    s.pref("interface enable-animations", "true")
    print(
        "Orca read Taskbar, Start, the launcher search, Quick Settings, the selected notification and a Settings switch.",
        flush=True,
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("output", type=Path)
    p.add_argument("--suite-target", required=True, type=Path)
    p.add_argument(
        "--only", choices=["launcher", "controls", "laptop", "forms", "appearance"]
    )
    opts = p.parse_args()
    s = Session(opts.run, opts.output)
    for name, check in [
        ("launcher", launcher),
        ("controls", controls),
        ("laptop", laptop),
        ("forms", forms),
        ("appearance", lambda s: appearance_and_speech(s, opts.suite_target)),
    ]:
        if not opts.only or name == opts.only:
            check(s)
