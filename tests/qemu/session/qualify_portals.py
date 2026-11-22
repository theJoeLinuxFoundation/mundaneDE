"""Qualify the desktop portals on a dedicated, headless try-mundane guest."""

import argparse
from pathlib import Path
import re
import shlex
import shutil
import time

from guest import Guest

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
args = parser.parse_args()
g = Guest(args.run)
here = Path(__file__).resolve().parent
fixtures = "/tmp/mundane-portal-test"
work = "/run/user/1000/mundane-portal-test"


def joe(command):
    env = "export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus WAYLAND_DISPLAY=wayland-1 XDG_CURRENT_DESKTOP=mundane XDG_SESSION_TYPE=wayland; "
    return g.shell("su joe -s /bin/sh -c " + shlex.quote(env + command))


def wait(label, check, timeout=25):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = check()
        if value:
            return value
        time.sleep(0.25)
    raise RuntimeError(label)


def widgets():
    return joe(f"python3 -W ignore {fixtures}/shell_widgets.py list").splitlines()


def button(label):
    return any(s in ("button " + label, "push button " + label) for s in widgets())


def click(label):
    wait("button " + label, lambda: button(label))
    joe(f"python3 -W ignore {fixtures}/shell_widgets.py click " + shlex.quote(label))


def launch(script, log, *args):
    joe(
        f"python3 -u {fixtures}/{script}.py {shlex.join(args)} </dev/null >{work}/{log}.log 2>&1 & echo $! > {work}/{log}.pid"
    )


def content(filename):
    return joe(f"cat {work}/{filename} 2>/dev/null || :")


def logged(log, phrase):
    return wait(f"{log}: {phrase}", lambda: phrase in content(log + ".log"))


def force_output(on):
    g.shell(
        f"echo {'on' if on else 'off'} >/sys/class/drm/card0-Virtual-2/status; echo change >/sys/class/drm/card0/uevent"
    )


def colors(name, head):
    path = g.run / (name + ".ppm")
    g.qmp("screendump", device="gpu", head=head, filename=str(path))
    fmt, dimensions, depth, data = path.read_bytes().split(b"\n", 3)
    assert fmt == b"P6" and depth == b"255"
    w, h = map(int, dimensions.split())
    return [
        data[i : i + 3].hex()
        for y in (h // 4, 3 * h // 4)
        for i in [(y * w + w // 2) * 3]
    ]


def near(actual, expected):
    return all(
        abs(x - y) <= 4
        for a, e in zip(actual, expected, strict=True)
        for x, y in zip(bytes.fromhex(a), bytes.fromhex(e), strict=True)
    )


green = ["19b333", "194ccc"]
red = ["cc1919", "8c1919"]


def has_video_node():
    # pw-dump reports the session PipeWire graph. xdpw chooses a unique media.name.
    return bool(joe("pw-dump | grep -F xdpw-stream- || :"))


def clean_stream():
    wait("PipeWire source removed", lambda: not has_video_node())
    wait(
        "sharing control removed",
        lambda: not any("button Stop sharing " in s for s in widgets()),
    )
    wait(
        "capture worker stopped",
        lambda: not joe("pgrep -u joe -f '^/usr/.*/xdg-desktop-portal-wlr -c ' || :"),
    )


def firefox():
    launch("firefox", "web")
    wait("WebRTC page", lambda: button("Share a screen"))
    # The page autofocuses the request button. Firefox's own chrome requires real input.
    g.key("ret")
    wait("Firefox permission prompt", lambda: button("Allow"))
    time.sleep(1.2)  # Firefox delays enabling its permission button.
    g.key("alt", "a")
    wait("desktop picker", lambda: button("Share Virtual-2"))


def shared(source="Virtual-2", expected=green):
    click("Share " + source)

    def received():
        for top, bottom in re.findall(
            r"FIREFOX pixels ([0-9,]+) ([0-9,]+)", content("web.log")
        ):
            if near(
                [bytes(map(int, c.split(","))).hex() for c in (top, bottom)], expected
            ):
                return True

    wait("Firefox received selected monitor's pixels", received)
    assert has_video_node()
    wait("visible sharing control", lambda: button("Stop sharing " + source))


def quit_browser():
    joe(f"kill -TERM $(cat {work}/browser.pid)")
    wait(
        "browser exited",
        lambda: not joe(f"ps -p $(cat {work}/browser.pid) -o comm= || :"),
    )


def setup():
    nodes = {n["node-name"]: n for n in g.qmp("query-named-block-nodes")}
    assert nodes["base"]["ro"] and nodes["basefile"]["ro"]
    assert Path(nodes["basefile"]["file"]).name == "joe-desktop-beta1.img"
    pid = int((g.run / "vm.pid").read_text())
    cmd = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
    assert cmd[cmd.index(b"-display") + 1] == b"none"
    assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
    for name in (
        "org.freedesktop.portal.Desktop",
        "org.freedesktop.impl.portal.desktop.gtk",
        "org.freedesktop.impl.portal.desktop.mundane",
    ):
        wait(
            name, lambda: name in joe("busctl --user --acquired --no-pager --full list")
        )
    assert "org.freedesktop.impl.portal.desktop.gtk" in g.shell(
        "journalctl -b --no-pager -g org.freedesktop.impl.portal.desktop.gtk"
    )
    for f in [*(here / "portals").glob("*.*"), here / "shell_widgets.py"]:
        shutil.copyfile(f, g.run / "share" / f.name)
    g.shell(
        f"set -eu; mkdir -p {fixtures}; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt; install -m644 /mnt/associate.py /mnt/client.py /mnt/firefox.py /mnt/patterns.py /mnt/uri_handler.py /mnt/org.joelinux.PortalTest.desktop /mnt/shell_widgets.py {fixtures}/; umount /mnt"
    )
    joe(
        f"mkdir -p {work} ~/.local/share/applications; cp {fixtures}/*.desktop ~/.local/share/applications/;  gio mime x-scheme-handler/mundane-portal-test org.joelinux.PortalTest.desktop; echo 'portal test' > {work}/example.txt; python3 {fixtures}/associate.py"
    )
    force_output(True)
    wait("two panel outputs", lambda: widgets().count("button Start") == 2)
    launch("patterns", "patterns")
    wait(
        "reference pattern on second virtual monitor",
        lambda: near(colors("portal-output", 1), green),
    )


def captures():
    launch("client", "shot", "screenshot")
    click("Take screenshot of Virtual-2")
    logged("shot", "IMAGE")
    assert content("shot.log").split()[-2:] == colors("portal-output", 1)
    count = lambda: joe(
        "find /run/user/1000 -maxdepth 1 -name 'mundane-screenshot-*.png' | wc -l"
    )
    before = count()
    launch("client", "shot", "screenshot")
    wait("screenshot consent", lambda: button("Take screenshot of Virtual-2"))
    click("Cancel")
    logged("shot", "Screenshot (1, {})")
    assert count() == before
    print("Screenshot grant, PNG pixels and cancellation passed", flush=True)
    firefox()
    click("Cancel")
    logged("web", "FIREFOX denied")
    assert "FIREFOX pixels" not in content("web.log")
    clean_stream()
    # Firefox covers the primary pattern. A window share must still return both red bands.
    assert not near(colors("portal-window-covered", 0), red)
    launch("client", "shot", "screenshot")
    click("Take screenshot of Pattern Virtual-1")
    logged("shot", "IMAGE")
    assert near(content("shot.log").split()[-2:], red)
    launch("client", "restricted", "cast")
    click("Share Pattern Virtual-1")
    logged("restricted", "WINDOW cc1919 8c1919")
    clean_stream()
    quit_browser()
    firefox()
    shared("Pattern Virtual-1", red)
    joe(f"kill -TERM $(cat {work}/patterns.pid)")
    logged("web", "FIREFOX ended")
    clean_stream()
    quit_browser()
    launch("patterns", "patterns")
    wait("patterns restored", lambda: near(colors("portal-output", 1), green))
    print(
        "Firefox cancellation, occluded window capture and window-close teardown passed",
        flush=True,
    )
    for action in ("stop", "browser-exit", "lock", "unplug"):
        firefox()
        shared()
        if action == "browser-exit":
            quit_browser()
        elif action == "stop":
            click("Stop sharing Virtual-2")
        elif action == "unplug":
            force_output(False)
        else:
            g.key("meta_l", "l")
        if action != "browser-exit":
            logged("web", "FIREFOX ended")
        if action == "lock":
            assert (
                g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes"
            )
            for line in content("web.log").splitlines():
                if line.startswith("FIREFOX pixels "):
                    palette = [
                        bytes(map(int, c.split(","))).hex() for c in line.split()[2:]
                    ]
                    assert near(palette, green), line
            for key in ("j", "o", "e", "ret"):
                g.key(key)
                time.sleep(0.15)
            wait(
                "PAM unlock",
                lambda: (
                    g.shell(f"loginctl show-session {sid} -p LockedHint --value")
                    == "no"
                ),
            )
        clean_stream()
        if action != "browser-exit":
            quit_browser()
        print("Firefox PipeWire pixels and " + action + " teardown passed", flush=True)


def gtk_portals():
    # GTK's chooser selects an exact file through its location entry.
    launch("client", "chooser", "choose")
    wait("GTK chooser", lambda: "file chooser Portal test file" in widgets())
    g.key("ctrl", "l")
    time.sleep(0.35)
    for ch in work + "/example.txt":
        g.key({"/": "slash", "-": "minus", ".": "dot"}.get(ch, ch))
        time.sleep(0.07)
    g.key("ret")
    logged("chooser", "OpenFile (0,")
    assert work + "/example.txt" in content("chooser.log")
    launch("client", "chooser", "choose")
    wait("GTK chooser", lambda: "file chooser Portal test file" in widgets())
    click("Cancel")
    logged("chooser", "OpenFile (1,")
    assert "file://" not in content("chooser.log")
    joe(f"rm -f {work}/opened")
    launch("client", "uri", "open")
    logged("uri", "OpenURI (0,")
    wait(
        "preferred handler",
        lambda: content("opened") == "mundane-portal-test://item/42",
    )
    launch("client", "notification", "notify")
    click("Review capture")
    logged("notification", "NOTIFICATION ACTION")
    assert "'review'" in content("notification.log")
    print(
        "GTK file selection and cancellation, preferred URI handler and notification action passed",
        flush=True,
    )


setup()
sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
captures()
gtk_portals()
joe(f"kill -TERM $(cat {work}/patterns.pid)")
