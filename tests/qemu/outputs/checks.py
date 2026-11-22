"""Exercises only the two virtio scanouts of the test guest."""

import json
import math
import os
from pathlib import Path
import re
import subprocess
import time

assert Path("/sys/class/dmi/id/product_name").read_text().strip().startswith("Standard PC")
assert os.environ["XDG_RUNTIME_DIR"] == "/run/test"
serial = next(Path("/dev").glob("vport*p1")).open("r+b", buffering=0)


def driver(*args):
    serial.write(json.dumps(args).encode() + b"\n")
    return json.loads(serial.readline())


def wait(desc, probe, timeout=12):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if r := probe():
            return r
        time.sleep(0.08)
    raise RuntimeError(desc)


def registry():
    return subprocess.check_output(["wayland-info"], text=True)


def connected(on):
    Path("/sys/class/drm/card0-Virtual-2/status").write_text("on" if on else "off")
    Path("/sys/class/drm/card0/uevent").write_text("change")
    wait("connector change", lambda: registry().count("interface: 'wl_output'") == (2 if on else 1))


def read(path):
    return Path(path).read_text(errors="replace")


def check_logs(file, pattern):
    return re.search(pattern, read(file))


def run_client(argv, name, **env):
    log = "/tmp/" + name + ".log"
    stream = open(log, "w")
    p = subprocess.Popen(
        ["setpriv", "--reuid=1000", "--regid=1000", "--groups=44,46", *argv],
        stdout=stream,
        stderr=stream,
        env=dict(os.environ, WAYLAND_DEBUG="client", **env),
    )
    stream.close()
    return p, log


def move(x, y, geometry):
    driver("move", *[round(max(0, min(v / lim, 1)) * 32767) for v, lim in zip((x, y), geometry)])
    time.sleep(0.12)


def click(button):
    driver("button", button, True)
    driver("button", button, False)


def screen(n, label):
    return driver("screen", n, label, "20df61")


def hotkey(k):
    driver("key", k, True)
    driver("key", k, False)


def output(*args):
    subprocess.run(
        [
            "setpriv",
            "--reuid=1000",
            "--regid=1000",
            "--groups=44,46",
            "mundane-de",
            "output",
            *map(str, args),
        ],
        check=True,
    )


connected(True)
present = subprocess.check_output(
    ["/test/nested-capabilities", "/run/test/wayland-1", "--presentation", "drm"],
    text=True,
    timeout=5,
)
flips = re.sub(r"\x1b\[[0-9;]*m", "", read("/tmp/compositor.log"))
pairs = re.findall(r"presented seq=(\d+) ns=(\d+)", present)
assert len(pairs) == 2 and all(f"seq={seq} ns={stamp}" in flips for seq, stamp in pairs), present
print(present, flush=True)
# Both CRTCs see the same crossfade and report their own kernel vblank clock.
start = len(read("/tmp/compositor.log"))
driver("key", "meta_l", True)
hotkey("w")
driver("key", "meta_l", False)
time.sleep(2)
trace = re.sub(r"\x1b\[[0-9;]*m", "", read("/tmp/compositor.log")[start:])
for name, hz in [("Virtual-1", 74.999), ("Virtual-2", 60.004)]:
    flips = [
        (int(seq), int(stamp))
        for seq, stamp in re.findall(rf'DRM page flip output="?{name}"? seq=(\d+) ns=(\d+)', trace)
    ]
    assert len(flips) >= 8, (name, trace[-1200:])
    intervals = [(b[1] - a[1]) / 1e6 for a, b in zip(flips, flips[1:])]
    ticks = [ms / (b[0] - a[0]) for a, b, ms in zip(flips, flips[1:], intervals)]
    assert abs(sorted(ticks)[len(ticks) // 2] - 1000 / hz) < 0.5, (name, ticks)
    print(
        name,
        "page-flip intervals in ms (n, p50, p95, p99)",
        len(intervals),
        [round(sorted(intervals)[math.ceil(len(intervals) * q) - 1], 3) for q in (0.5, 0.95, 0.99)],
        flush=True,
    )
reg = registry()
print(reg[reg.index("interface: 'wl_output'") :][:1400], flush=True)
gtk, log = run_client(["python", "/test/scale_client.py"], "gtk", GSK_RENDERER="gl")
wait("GTK map", lambda: check_logs(log, r"attach\(wl_buffer"))
time.sleep(1.3)
first = wait(
    "settled GTK rectangle",
    lambda: a if (a := screen(0, "gtk-100"))["box"] == [300, 240, 500, 360] else None,
)
print("first", first, flush=True)
assert first["size"] == [800, 600] and first["box"] == [300, 240, 500, 360], first
# 1024/1.5 rounds to 683 logical pixels. The first monitor remains 800x600.
bounds = (1483, 600)
move(360, 290, bounds)
click("left")
wait("exact click", lambda: check_logs(log, r"hit (59|60)\.\d (49|50)\.\d"))
driver("key", "alt", True)
driver("button", "left", True)
move(650, 265, bounds)
move(920, 260, bounds)
move(1080, 220, bounds)
during = screen(1, "drag")
# The pointer holds the (60,50) logical hit point. The tinted preview covers
# the overlap with the future target, leaving the left edge in its original colour.
assert during["box"][:2] == [330, 255], during
driver("button", "left", False)
driver("key", "alt", False)
time.sleep(1.2)
print("drag and settled", during, screen(1, "gtk-150"), flush=True)
assert check_logs(log, r"preferred_scale\(180\)")
second = screen(1, "gtk-150")
x, y, r, b = second["box"]
assert (x, y, r - x, b - y) == (362, 255, 300, 180), second
move(800 + (x + 45) / 1.5, (y + 45) / 1.5, bounds)
click("left")
wait("scaled click", lambda: len(re.findall(r"^hit ", read(log), re.M)) >= 2)
hits = re.findall(r"^hit ([.\d]+) ([.\d]+)", read(log), re.M)
print("scaled hit", hits[-1], flush=True)
assert all(abs(float(v) - 30) <= 1 for v in hits[-1]), hits
# The preferred mode is the first current mode of Virtual-2.
block = next(part for part in reg.split("interface: 'wl_output'") if "model: 'Virtual-2'" in part)
print(block[:500], flush=True)
current = re.search(
    r"width: (\d+) px, height: (\d+) px, refresh: ([.\d]+) Hz,\s+flags:[^\n]*current", block
)
assert current, block
mode = [int(current[1]), int(current[2]), round(float(current[3]) * 1000)]
print("Mode on Virtual-2", mode, flush=True)


def trial(scale, mode=mode, rotation=0):
    output("test", "Virtual-2", *mode, scale, rotation)


def scale_seen(factor, offset):
    return f"preferred_scale({factor})" in read(log)[offset:]


try:
    output("test", "Virtual-2", 12, 17, 60000, 2)
except subprocess.CalledProcessError:
    assert screen(1, "invalid-mode")["box"] == second["box"]
else:
    raise AssertionError("accepted an unadvertised mode")
offset = len(read(log))
trial(2)
wait("200% scale", lambda: scale_seen(240, offset))
time.sleep(0.5)


def double_frame():
    image = screen(1, "gtk-200")
    box = image["box"]
    return image if box and (box[2] - box[0], box[3] - box[1]) == (400, 240) else None


big = wait("400x240 scanout rectangle", double_frame)
print("at double scale", big, flush=True)
for sample in (first, second, big):
    assert sample["stripes"] == [0, 223] * 8, sample
assert 20 < first["glyph_edges"] < second["glyph_edges"] < big["glyph_edges"]
left, top, right, bottom = big["box"]
assert (right - left, bottom - top) == (400, 240)
offset = len(read(log))
output("revert")
wait("scale after rollback", lambda: scale_seen(180, offset))
# Change resolution and rotate. Let the session deadline restore both mode and layout.
alt = re.search(r"width: (800) px, height: (600) px, refresh: ([\d.]+) Hz", block)
assert alt
trial(2, [800, 600, round(float(alt[3]) * 1000)], 90)
wait("trial scanout mode", lambda: screen(1, "trial")["size"] == [800, 600])
wait("timed rollback", lambda: screen(1, "reverted")["size"] == [1024, 768], 20)
# Return to the pointer at a reachable position after removing its current head.
connected(False)
wait("restored focus and window on survivor", lambda: screen(0, "unplugged")["box"])
hotkey("a")
wait("keyboard focus after unplug", lambda: "key 97" in read(log))
connected(True)
assert "logical_x: 800" in registry()
# With reduced motion enabled repeat the same release on the secondary display.
config = Path(os.environ["XDG_CONFIG_HOME"]) / "mundane/config.toml"
config.write_text("reduced_motion=true\n[displays.Virtual-2]\nscale=1.5\n")
time.sleep(0.4)
home = screen(0, "retained")["box"]
x, y = home[:2]
move(x + 60, y + 50, bounds)
driver("key", "alt", True)
driver("button", "left", True)
move(1080, 220, bounds)
driver("button", "left", False)
driver("key", "alt", False)


def released_box():
    box = screen(1, "reduced")["box"]
    return box if box and box[2] - box[0] == 300 else None


assert wait("first frame after snap preview clears", released_box) == second["box"]
# Move to the right edge and request a right-opening popup.
move(1080, 220, bounds)
driver("key", "alt", True)
driver("button", "left", True)
move(1400, 190, bounds)
driver("button", "left", False)
driver("key", "alt", False)
edge = screen(1, "edge-window")["box"]
assert edge
move(800 + (edge[2] - 9) / 1.5, (edge[1] + 45) / 1.5, bounds)
click("right")
wait("edge popup on second output", lambda: driver("screen", 1, "popup", "db238a")["box"])
assert check_logs(log, r"xdg_popup.*configure\(")
hotkey("esc")
wait("popup dismissed", lambda: driver("screen", 1, "dismissed", "db238a")["box"] is None)
gtk.terminate()
gtk.wait()


def move_app(name, argv, color, **env):
    move(400, 300, bounds)
    app, trace = run_client(argv, name, **env)
    try:
        wait(name + " maps", lambda: check_logs(trace, r"attach\(wl_buffer"), 30)

        def scene(head, label):
            return driver("screen", head, name + "-" + label, color)["box"]

        box = wait(name + " test colour at 100%", lambda: scene(0, "100"))
        # Alt+left moves by the pointer offset even for a large client.
        px, py = (box[0] + box[2]) // 2, (box[1] + box[3]) // 2
        move(px, py, bounds)
        driver("key", "alt", True)
        driver("button", "left", True)
        move(1430, 240, bounds)
        driver("button", "left", False)
        driver("key", "alt", False)
        wait(name + " gets 150%", lambda: check_logs(trace, r"preferred_scale\(180\)"))
        wait(name + " test colour at 150%", lambda: scene(1, "150"))
        trial(2)
        wait(name + " gets 200%", lambda: check_logs(trace, r"preferred_scale\(240\)"))
        wait(name + " test colour at 200%", lambda: scene(1, "200"))
        output("revert")
        print(name + " mapped at 100%, 150% and 200%", flush=True)
    finally:
        app.terminate()
        app.wait(timeout=8)


doc = Path("/tmp/page.html")
doc.write_text('<body style="margin:0;background:#20df61"><h1>Display 123</h1></body>')
profile = Path("/home/test/firefox")
profile.mkdir()
os.chown(profile, 1000, 1000)
(profile / "user.js").write_text(
    'user_pref("browser.aboutwelcome.enabled",false);\nuser_pref("browser.shell.checkDefaultBrowser",false);\nuser_pref("browser.startup.homepage_override.mstone","ignore");\n'
)
move_app(
    "firefox",
    [
        "firefox",
        "--no-remote",
        "--profile",
        str(profile),
        "--width",
        "480",
        "--height",
        "320",
        doc.as_uri(),
    ],
    "20df61",
    MOZ_ENABLE_WAYLAND="1",
)
move_app(
    "gpui",
    ["mundane-files"],
    "17201e",
    VK_DRIVER_FILES=str(next(Path("/usr/share/vulkan/icd.d").glob("lvp_icd*.json"))),
    WGPU_BACKEND="vulkan",
)
output("test", "Virtual-2", *mode, 2, 0, "--position", "-512", "0")
wait("negative logical output origin", lambda: "logical_x: -512" in registry())


def confirm():
    cmd = [
        "setpriv",
        "--reuid=1000",
        "--regid=1000",
        "--groups=44,46",
        "mundane-de",
        "output",
        "confirm",
    ]
    return subprocess.run(cmd, capture_output=True).returncode == 0


wait("confirmed after the first page flip", confirm)
connected(False)
connected(True)
reg = registry()
assert "logical_x: -512" in reg and "logical_width: 512" in reg
print("confirmed mode, scale and position survived reconnect", flush=True)
