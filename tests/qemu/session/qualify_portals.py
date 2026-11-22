"""Qualify the desktop portals on a dedicated, headless try-mundane guest."""

import argparse
import re
import shlex
import shutil
import subprocess
import time
from pathlib import Path

from guest import Guest

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
parser.add_argument(
    "--gpui-screenshots", type=Path, help="test the GPUI preview and save its scanouts"
)
parser.add_argument(
    "--flatpak-client",
    type=Path,
    help="exercise a document grant from an isolated Flatpak using the compiled portals/grant.c client",
)
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


def shot(name):
    if args.gpui_screenshots:
        args.gpui_screenshots.mkdir(exist_ok=True, parents=True)
        image = g.screen(name)
        assert image.startswith(b"P6\n1920 1080\n255\n")
        with (args.gpui_screenshots / (name + ".png")).open("wb") as output:
            subprocess.run(["pnmtopng"], input=image, stdout=output, check=True)


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
yellow = ["e6cc19", "b36633"]


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


def received_pixels(expected, after=0):

    def received():
        for top, bottom in re.findall(
            r"FIREFOX pixels ([0-9,]+) ([0-9,]+)", content("web.log")[after:]
        ):
            if near(
                [bytes(map(int, c.split(","))).hex() for c in (top, bottom)], expected
            ):
                return True

    wait("Firefox received selected pixels", received)


def shared(source="Virtual-2", expected=green):
    click("Share " + source)
    received_pixels(expected)
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
    if args.gpui_screenshots:
        wait("GPUI login", lambda: g.shell("pgrep -u joe -x mundane-shell || :"))
        joe(
            "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true"
        )
    for name in (
        "org.freedesktop.portal.Desktop",
        "org.freedesktop.impl.portal.desktop.gtk",
        "org.freedesktop.impl.portal.desktop.mundane",
    ):
        wait(
            name,
            lambda name=name: (
                name in joe("busctl --user --acquired --no-pager --full list")
            ),
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


def appearance_after_restart():
    read = "gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop --method org.freedesktop.portal.Settings.Read org.freedesktop.appearance color-scheme"
    wait("appearance service after restart", lambda: "uint32" in joe(read + " || :"))
    setting = "org.gnome.desktop.interface color-scheme"
    original = joe("gsettings get " + setting)
    value = "prefer-light" if "prefer-dark" in original else "prefer-dark"
    monitor = int(
        joe(
            f"gdbus monitor --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop >{work}/appearance.log 2>&1 </dev/null & echo $!"
        )
    )
    try:
        wait(
            "appearance signal listener",
            lambda: "owned by" in content("appearance.log"),
        )
        joe(f"gsettings set {setting} {value}")
        wait(
            "updated appearance signal",
            lambda: (
                "SettingChanged ('org.freedesktop.appearance', 'color-scheme'"
                in content("appearance.log")
            ),
        )
        assert "uint32 " + ("2" if value == "prefer-light" else "1") in joe(read)
    finally:
        joe(f"gsettings set {setting} {original}; kill {monitor} || :")


def captures():
    launch("client", "shot", "screenshot")
    wait("screenshot picker", lambda: button("Take screenshot of Virtual-2"))
    shot("screenshot-consent")
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
    cases = ["stop", "browser-exit", "lock", "unplug"]
    if args.gpui_screenshots:
        cases[1:1] = ["shell-exit", "frontend-exit"]
    for action in cases:
        firefox()
        if action == "stop":
            shot("share-consent")
        shared()
        if args.gpui_screenshots:
            wait(
                "Stop on both taskbars",
                lambda: widgets().count("button Stop sharing Virtual-2") == 2,
            )
        if action == "stop":
            shot("active-share")
        if action in ("shell-exit", "frontend-exit"):
            # Prove the browser follows fresh monitor damage before revocation.
            flip = f"kill -USR1 $(cat {work}/patterns.pid)"
            start = len(content("web.log"))
            joe(flip)
            received_pixels(yellow, start)
            start = len(content("web.log"))
            joe(flip)
            received_pixels(green, start)
        if action == "browser-exit":
            quit_browser()
        elif action == "stop":
            click("Stop sharing Virtual-2")
        elif action == "shell-exit":
            pid = int(g.shell("pgrep -u joe -f '^/usr/local/bin/mundane-shell$'"))
            g.shell(f"kill -KILL {pid}")
        elif action == "frontend-exit":
            joe("systemctl --user restart xdg-desktop-portal.service")
        elif action == "unplug":
            force_output(False)
        else:
            g.key("meta_l", "l")
        if action not in ("browser-exit", "frontend-exit"):
            logged("web", "FIREFOX ended")
        if action == "lock":
            wait(
                "locker has covered the seat",
                lambda: (
                    g.shell(f"loginctl show-session {sid} -p LockedHint --value")
                    == "yes"
                ),
            )
            for line in content("web.log").splitlines():
                if line.startswith("FIREFOX pixels "):
                    palette = [
                        bytes(map(int, c.split(","))).hex() for c in line.split()[2:]
                    ]
                    assert near(palette, green), line
            if args.gpui_screenshots:
                wait(
                    "GPUI locker accepts input",
                    lambda: "password text Password" in widgets(),
                )
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
        if action in ("frontend-exit", "shell-exit"):
            start = len(content("web.log"))
            joe(flip)
            wait(
                "changed source after revocation",
                lambda: near(colors("revoked-output", 1), yellow),
            )
            appearance_after_restart()
            assert "FIREFOX pixels" not in content("web.log")[start:]
            joe(flip)
            wait(
                "original source restored",
                lambda: near(colors("restored-output", 1), green),
            )
        if action != "browser-exit":
            quit_browser()
        print("Firefox PipeWire pixels and " + action + " teardown passed", flush=True)


def choice_portals():
    chooser = (
        "frame Portal test file"
        if args.gpui_screenshots
        else "file chooser Portal test file"
    )
    # Select an exact file through the public portal and its location field.
    launch("client", "chooser", "choose")
    wait("file chooser", lambda: chooser in widgets())
    if args.gpui_screenshots:
        assert "button File type: Text files" in widgets()
        assert (
            "button example.txt" in widgets() and "button patterns.log" not in widgets()
        )
        shot("open-file")
    g.key("ctrl", "l")
    time.sleep(0.35)
    for ch in work + "/example.txt":
        g.key({"/": "slash", "-": "minus", ".": "dot"}.get(ch, ch))
        time.sleep(0.07)
    g.key("ret")
    logged("chooser", "OpenFile (0,")
    assert work + "/example.txt" in content("chooser.log")
    launch("client", "chooser", "choose")
    wait("file chooser", lambda: chooser in widgets())
    click("Cancel")
    logged("chooser", "OpenFile (1,")
    assert "file://" not in content("chooser.log")
    if args.gpui_screenshots:
        launch("client", "save", "save")
        wait("GPUI Save", lambda: "frame Portal save test" in widgets())
        click("Save")
        click("Replace")
        logged("save", "SaveFile (0,")
        assert work + "/example.txt" in content("save.log")
        launch("client", "save-many", "save-many")
        wait("GPUI destination folder", lambda: "frame Portal save test" in widgets())
        click("Save")
        logged("save-many", "SaveFiles (0,")
        assert "second.txt" in content(
            "save-many.log"
        ) and "example%20(1).txt" in content("save-many.log").replace(
            "%28", "("
        ).replace("%29", ")")
        launch("client", "ask", "ask")
        wait("GPUI application choice", lambda: "frame Open with" in widgets())
        shot("open-with")
        click("Portal test handler")
        logged("ask", "OpenURI (0,")
        wait(
            "chosen URI handler",
            lambda: content("opened") == "mundane-portal-test://item/43",
        )
    if args.flatpak_client:
        shutil.copyfile(args.flatpak_client, g.run / "share" / "file-grant")
        g.shell(
            f"mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && install -m755 /mnt/file-grant /mnt/grant.sh {fixtures}/ && umount /mnt"
        )
        joe(
            f"sh {fixtures}/grant.sh {fixtures}/file-grant </dev/null >{work}/sandbox.log 2>&1 &"
        )
        wait("sandbox file chooser", lambda: "frame Sandbox file" in widgets(), 60)
        click("example.txt")
        click("Open")
        logged("sandbox", "readable 1")
        assert "Document URI file:///run/user/1000/doc/" in content("sandbox.log")
        print("Flatpak read the selected file through its document grant.", flush=True)
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
        "File selection and cancellation, preferred URI handler and notification action passed",
        flush=True,
    )


setup()
sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
captures()
choice_portals()
force_output(False)
joe(f"kill -TERM $(cat {work}/patterns.pid)")
