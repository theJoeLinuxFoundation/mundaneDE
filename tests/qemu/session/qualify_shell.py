"""Qualify milestone 5 on a dedicated headless 1920x1080 try-mundane overlay."""

from pathlib import Path
import argparse
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


def launch(what, logfile):
    joe(f"({what}) </dev/null >{logfile} 2>&1 &")


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
wait("shell", lambda: g.shell("pgrep -x -u joe mundane-shell || :"))
joe(
    "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true"
)
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
click("Workspaces, current 1")
click("Move →")
wait("window moved to another workspace", lambda: rgb not in ppm("desk-1")[1])
g.key("meta_l", "2")
wait(
    "window on second workspace", lambda: ppm("desk-2")[1].count(rgb) >= 320 * 170 - 500
)
pid = g.shell("pgrep -u joe -x mundane-shell || :")
g.shell("kill " + pid)
wait(
    "replacement panel",
    lambda: (p := g.shell("pgrep -u joe -x mundane-shell || :")) and p != pid,
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
click("Workspaces, current 1")
wait("overview opened", lambda: "Desk 2" in widget("list"))
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
    joe(
        f"pactl load-module module-null-sink sink_name={name} sink_properties=device.description={name}"
    )
    for name in ("mundane_first", "mundane_second")
]
try:
    click("System menu")
    radio = joe("nmcli radio wifi")
    wait(
        "Wi-Fi tile ready",
        lambda: (
            "toggle button Wi-Fi" in widget("list").splitlines()
            and widget("description", "Wi-Fi") in ("On", "Off")
        ),
    )
    widget("click", "Wi-Fi")
    wait("NetworkManager radio toggled", lambda: joe("nmcli radio wifi") != radio)
    changed = joe("nmcli radio wifi")
    wait(
        "Wi-Fi state reflected",
        lambda: (
            widget("description", "Wi-Fi").strip()
            == ("On" if changed == "enabled" else "Off")
        ),
    )
    widget("click", "Wi-Fi")
    wait("NetworkManager radio restored", lambda: joe("nmcli radio wifi") == radio)
    active_sink = joe("wpctl inspect @DEFAULT_AUDIO_SINK@")
    other = "mundane_first" if "mundane_second" in active_sink else "mundane_second"
    widget("click", "Devices and connections")
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

# Lock, notification history, OSD focus and deployment notices have dedicated guest gates.
