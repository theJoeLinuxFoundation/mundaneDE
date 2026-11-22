"""Qualify milestone 5 on a dedicated headless 1920x1080 try-mundane overlay."""

from pathlib import Path
import argparse
import json
import hashlib
import shlex
import shutil
import time

from guest import Guest

here = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
args = parser.parse_args()
g = Guest(args.run)
runtime = "/run/user/1000"
display = runtime + "/wayland-1"
log = runtime + "/mundane-login-test.log"


def wait(label, fn, seconds=12):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = fn()
        if value:
            return value
        time.sleep(0.12)
    raise RuntimeError(label)


def joe(line):
    env = "export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus WAYLAND_DISPLAY=wayland-1; "
    return g.shell("su joe -s /bin/sh -c " + shlex.quote(env + line))


def widget(verb, *words):
    return joe(
        "python3 -W ignore /tmp/mundane-shell-test/shell_widgets.py "
        + shlex.join([verb, *words])
    )


def click(name):
    wait("button " + name, lambda: "button " + name in widget("list").splitlines())
    widget("click", name)


def keys(*names):
    for n in names:
        g.key(n)
        time.sleep(0.13)


def ppm(label, head=0):
    path = g.run / (label + ".ppm")
    g.qmp("screendump", device="gpu", head=head, filename=str(path))
    magic, dims, depth, pixels = path.read_bytes().split(b"\n", 3)
    assert magic == b"P6" and depth == b"255"
    return tuple(map(int, dims.split())), pixels


def protected(label, n, inactive=None):
    _, pixels = ppm(label, n)
    if pixels == inactive:
        return False  # QEMU is still displaying its own disconnected-head message.
    # An idle PAM surface and the compositor's independent fallback are both opaque.
    assert pixels[:3] in (bytes.fromhex("182536"), bytes.fromhex("0a121a"))
    assert pixels == pixels[:3] * (len(pixels) // 3)
    return True


def force_output(n, on):
    assert n in (2, 3)
    g.shell(
        f"echo {'on' if on else 'off'} >/sys/class/drm/card0-Virtual-{n}/status; echo change >/sys/class/drm/card0/uevent"
    )


def launch(what, logfile):
    joe(f"({what}) </dev/null >{logfile} 2>&1 &")


def logtext():
    return g.shell("cat " + log + " 2>/dev/null || :")


def authed(sid):
    keys("j", "o", "e", "ret")
    wait(
        "PAM unlock",
        lambda: g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "no",
    )


assert g.qmp("query-status")["running"]
assert all(
    n["ro"]
    for n in g.qmp("query-named-block-nodes")
    if n["node-name"] in ("base", "basefile")
)
assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
sid = wait(
    "GDM user session",
    lambda: (
        sid
        if (
            sid := g.shell("loginctl show-seat seat0 -p ActiveSession --value")
        ).isdigit()
        else None
    ),
    60,
)
assert g.shell(f"loginctl show-session {sid} -p Name --value") == "joe"
wait("panel", lambda: g.shell("pgrep -u joe -f '/mundane/shell/panel.py' || :"))
share = g.run / "share"
for source in [
    here / "shell_widgets.py",
    here / "shell_bus_fixture.py",
    here.parents[2] / "target/release/examples/nested-capabilities",
]:
    shutil.copyfile(source, share / source.name)
g.shell(
    "set -eu; mkdir -p /tmp/mundane-shell-test; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt; "
    "install -m755 /mnt/shell_*.py /mnt/nested-capabilities /tmp/mundane-shell-test/; chmod +x /tmp/mundane-shell-test/nested-capabilities; umount /mnt"
)
joe("gsettings set org.gnome.desktop.interface toolkit-accessibility true")
g.shell(
    "pkill -u joe -f '^/tmp/mundane-shell-test/nested-capabilities' || :; pkill -u joe -f '^python3 /tmp/mundane-shell-test/shell_bus_fixture.py' || :"
)
joe("rm -f /run/user/1000/shell-bus-events")
g.key("meta_l", "1")
g.key("esc")
time.sleep(0.2)
wait("Home on desktop", lambda: "button Home" in widget("list"))
assert ppm("desktop-layout")[0] == (1920, 1080)
# Launch through the actual searchable menu, then through the XDG Desktop directory.
g.move(0.25, 0.7)
time.sleep(0.2)
g.click(0.026, 0.981)
wait("Start search", lambda: "button Window overview" in widget("list"))
keys("t", "e", "x", "t", "spc", "e", "d", "i", "t", "o", "r")
time.sleep(0.45)
keys("ret")
wait(
    "Text Editor from Start",
    lambda: (
        joe("pgrep -u joe -f '^/usr/bin/gnome-text-editor' || :")
        and "application gnome-text-editor" in widget("list")
    ),
)
g.key("meta_l", "q")
joe("gio mime text/plain org.gnome.TextEditor.desktop")
joe("printf 'Shell file launch\n' > ~/Desktop/Mundane-check.txt")
wait("desktop file icon", lambda: "button Mundane-check.txt" in widget("list"))
g.key("meta_l", "d")
time.sleep(0.25)
keys("down", "down", "ret")
wait(
    "file opened",
    lambda: (
        "frame Mundane-check.txt" in widget("list")
        and "application gnome-text-editor" in widget("list")
    ),
)
g.key("meta_l", "q")
print(
    "Bottom taskbar, keyboard Start search and XDG Desktop file launch passed",
    flush=True,
)

# A real client negotiates SSD, creates a sandbox connection and inhibits idle while visible.
work = joe("mktemp -d /run/user/1000/shell-probe.XXXXXX")
probe = work + "/client.log"
launch(f"/tmp/mundane-shell-test/nested-capabilities {display} --shell {work}", probe)
wait(
    "client mapped", lambda: "server-decorated client mapped" in g.shell("cat " + probe)
)
wait("task list", lambda: "button Shell test window" in widget("list"))
time.sleep(2.1)
assert "idle event Idled" not in g.shell("cat " + probe)
(_, _), pixels = ppm("ssd-normal")
rgb = bytes.fromhex("bbaa44")
offset = pixels.find(rgb) // 3
assert offset >= 0
x, y = offset % 1920, offset // 1920
# The client resizes its buffer to each configure.
g.click((x + 320 - 45) / 1920, (y - 15) / 1080)
wait(
    "maximize up to bottom taskbar",
    lambda: (
        "configure 1920 100" in g.shell("cat " + probe)
        or any(
            l.startswith("configure 1920 ") and 900 < int(l.split()[2]) < 1050
            for l in g.shell("cat " + probe).splitlines()
        )
    ),
)
# Check the scanout reaches both edges and leaves the panel visible.
time.sleep(0.3)
_, pixels = ppm("ssd-maximized")
offset = pixels.find(rgb) // 3
x, y = offset % 1920, offset // 1920
assert x == 0 and y == 30 and pixels.count(rgb) > 1920 * 900
g.click((1920 - 45) / 1920, (y - 15) / 1080)
wait("SSD restore bounds", lambda: "configure 320 170" in g.shell("cat " + probe))
time.sleep(0.25)
_, pixels = ppm("ssd-restored")
offset = pixels.find(rgb) // 3
x, y = offset % 1920, offset // 1920
g.click((x + 320 - 75) / 1920, (y - 15) / 1080)
wait(
    "hidden inhibitor allows idle",
    lambda: "idle event Idled" in g.shell("cat " + probe),
)
assert rgb not in ppm("ssd-minimized")[1]
click("Shell test window")
wait(
    "restored from taskbar",
    lambda: ppm("ssd-task-restore")[1].count(rgb) >= 320 * 170 - 500,
)
g.key("meta_l", "spc")
click("Window overview")
click("Move →")
wait("window moved to another workspace", lambda: rgb not in ppm("desk-1")[1])
g.key("meta_l", "2")
wait(
    "window on second workspace", lambda: ppm("desk-2")[1].count(rgb) >= 320 * 170 - 500
)
pid = g.shell("pgrep -u joe -f '/mundane/shell/panel.py' | head -1")
g.shell("kill " + pid)
wait(
    "replacement panel",
    lambda: (
        (p := g.shell("pgrep -u joe -f '/mundane/shell/panel.py' | head -1"))
        and p != pid
    ),
)
wait("same task after restart", lambda: "button Shell test window" in widget("list"))
assert ppm("desk-2-after-restart")[1].count(rgb) >= 320 * 170 - 500
g.key("meta_l", "1")
print(
    "SSD controls, visible idle inhibition, sandbox registry, workspace move and panel restart passed",
    flush=True,
)

# A second window of the same desktop ID must remain independently selectable.
second = joe("mktemp -d /run/user/1000/shell-probe.XXXXXX")
launch(
    f"/tmp/mundane-shell-test/nested-capabilities {display} --shell {second} Companion",
    second + "/client.log",
)
wait("second window", lambda: "button Companion" in widget("list"))
g.key("meta_l", "spc")
click("Window overview")
wait("overview opened", lambda: "button Desk 2" in widget("list"))
keys("w", "i", "n", "d", "o", "w")
time.sleep(0.35)
keys("ret")
wait(
    "activate searched window on desk two",
    lambda: (
        "button Shell test window" in (rows := widget("list"))
        and "button Companion" not in rows
    ),
)
g.key("meta_l", "1")
wait("companion stayed on desk one", lambda: "button Companion" in widget("list"))
g.key("meta_l", "q")
wait(
    "close correct window",
    lambda: "close requested" in g.shell("cat " + second + "/client.log"),
)
g.key("meta_l", "2")
wait("first window survived", lambda: "button Shell test window" in widget("list"))
g.key("meta_l", "1")
print(
    "Two windows with the same application ID searched, activated and closed independently",
    flush=True,
)

# A D-Bus app owns a real tray item and notification action.
launch("python3 /tmp/mundane-shell-test/shell_bus_fixture.py", work + "/bus.log")
for control, event in [
    ("Tray test", "tray activated"),
    ("Open review", "notification action"),
]:
    click(control)
    wait(
        event,
        lambda: event in joe("cat /run/user/1000/shell-bus-events 2>/dev/null || :"),
    )
# Virtual radio and two real PipeWire null sinks in the guest.
g.shell("modprobe mac80211_hwsim")
modules = [
    joe("pactl load-module module-null-sink sink_name=" + name)
    for name in ("mundane_first", "mundane_second")
]
try:
    click("System menu")
    radio = joe("nmcli radio wifi")
    click("Wi-Fi " + radio)
    wait("NetworkManager radio toggled", lambda: joe("nmcli radio wifi") != radio)
    changed = joe("nmcli radio wifi")
    click("Wi-Fi " + changed)
    wait("NetworkManager radio restored", lambda: joe("nmcli radio wifi") == radio)
    active_sink = joe("wpctl inspect @DEFAULT_AUDIO_SINK@")
    other = "mundane_first" if "mundane_second" in active_sink else "mundane_second"
    click(other)
    wait(
        "WirePlumber changed sink",
        lambda: other in joe("wpctl inspect @DEFAULT_AUDIO_SINK@"),
    )
    keys("esc")
finally:
    for module in modules:
        joe("pactl unload-module " + module)
print("Virtual Wi-Fi toggle and WirePlumber sink selection passed", flush=True)
# Use a visible input probe to check OSD focus and locked input delivery.
joe(
    "busctl --user call org.joelinux.MundaneSessionTest /org/joelinux/MundaneSessionTest org.freedesktop.Application Activate 'a{sv}' 0"
)
wait("keyboard probe visible", lambda: "ready " in logtext())
keys("a")
wait("keyboard probe focused", lambda: "key a" in logtext())
g.key("volumeup")
keys("b")
wait("volume OSD kept focus", lambda: "key b" in logtext())
print("Tray activation, notification action and OSD focus passed", flush=True)

force_output(2, True)
time.sleep(0.5)
g.key("meta_l", "l")
wait(
    "locked hint",
    lambda: g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes",
)
for i in (0, 1):
    wait("covered output", lambda i=i: protected(f"locked-{i}", i))
inactive = ppm("disconnected-third-output", 2)[1]
force_output(3, True)
wait("new output covered", lambda: protected("locked-hotplug", 2, inactive))
g.shell("pkill -KILL -x swaylock")
for i in range(3):
    assert protected(f"locker-loss-{i}", i)
wait("restarted locker", lambda: g.shell("pgrep -u joe -x swaylock || :"))
before = logtext()
keys("w", "r", "o", "n", "g", "ret")
time.sleep(2)
assert (
    logtext() == before
    and g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes"
)
g.shell("systemctl suspend --no-block")
wait("suspended", lambda: g.qmp("query-status")["status"] == "suspended")
g.qmp("system_wakeup")
g.sync_console()
assert g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes"
authed(sid)
force_output(3, False)
force_output(2, False)
keys("d")
wait("app receives input again", lambda: "key d" in logtext())
print(
    "Two locked outputs, hotplug of a third, locker loss, refused password, S3 and PAM unlock passed",
    flush=True,
)

# Put recorded dave.status/1 documents on the panel's PATH only. The beta1
# image predates r3. The real installed dave and its deployment store stay intact.
from upgrade_checks import snapshot

fixture = "/tmp/mundane-shell-test"
g.shell(
    "set -eu; mkdir -p "
    + fixture
    + "/bin; install -m755 /usr/local/bin/mundane-panel "
    + fixture
    + "/panel.orig; "
    "printf '%s\\n' '#!/bin/sh' 'test \"$*\" = \"status --json\" || exit 1' 'exec cat /tmp/mundane-shell-test/status.json' > "
    + fixture
    + "/bin/dave; "
    "chmod 755 "
    + fixture
    + "/bin/dave; sed '2i export PATH=/tmp/mundane-shell-test/bin:$PATH' "
    + fixture
    + "/panel.orig > /usr/local/bin/mundane-panel"
)


def publish(status):
    g.shell(
        "printf '%s' "
        + shlex.quote(json.dumps(status))
        + " > "
        + fixture
        + "/pending && mv "
        + fixture
        + "/pending "
        + fixture
        + "/status.json"
    )


def restart_panel():
    pid = g.shell("pgrep -u joe -f '/mundane/shell/panel.py' | head -1")
    g.shell("kill " + pid)
    wait(
        "panel restarted",
        lambda: (
            (p := g.shell("pgrep -u joe -f '/mundane/shell/panel.py' | head -1"))
            and p != pid
        ),
    )
    wait("tray watcher ready", lambda: "button Start" in widget("list"))


try:
    publish(snapshot(1))
    restart_panel()
    stage = {
        "id": "sha256:" + hashlib.sha256(work.encode()).hexdigest(),
        "install": 2,
        "remove": 0,
        "summary": "2 to install or upgrade, 0 to remove",
    }
    publish(snapshot(2, stage))
    wait(
        "prepared notification",
        lambda: "label Upgrade ready, restart to apply" in widget("list").splitlines(),
        22,
    )
    restart_panel()
    time.sleep(1)
    assert "label Upgrade ready, restart to apply" not in widget("list").splitlines()
    click("System menu")
    wait("prepared status retained", lambda: stage["summary"] in widget("list"))
    keys("esc")
    publish(snapshot(3, outcome="not_running"))
    time.sleep(16)
    click("System menu")
    time.sleep(0.4)
    rows = widget("list")
    assert (
        "Upgrade ready, restart to apply" not in rows
        and "Upgrade to deployment 5 failed" not in rows
    )
    keys("esc")
    failed = snapshot(4, outcome="fallback")
    failed["last_upgrade"]["event"] = "boot-" + work.rsplit("/", 1)[1]
    publish(failed)
    wait(
        "automatic fallback toast",
        lambda: "Returned to deployment 4." in widget("list"),
        22,
    )
    restart_panel()
    time.sleep(1)
    assert "Returned to deployment 4." not in widget("list")
    click("System menu")
    wait(
        "persistent recovery details",
        lambda: (
            "Upgrade to deployment 5 failed" in (details := widget("list"))
            and "Returned to deployment 4." in details
        ),
    )
    g.screen("recovery-details")
    keys("esc")
finally:
    g.shell("install -m755 " + fixture + "/panel.orig /usr/local/bin/mundane-panel")
    restart_panel()
print(
    "dave staged and fallback notifications, manual rollback suppression and restart deduplication passed",
    flush=True,
)
