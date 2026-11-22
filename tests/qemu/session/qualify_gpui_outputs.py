"""Check GPUI layer admission, exclusive areas and output lifetimes in the joe guest."""

import argparse
import json
import re
import shlex
import shutil
from pathlib import Path

from guest import Guest
from qualify_menus import wait


def joe(g, command):
    return g.shell(
        "su joe -s /bin/sh -c "
        + shlex.quote(
            "export PATH=/usr/local/bin:$PATH XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; "
            "export WAYLAND_DISPLAY=$(systemctl --user show-environment | sed -n 's/^WAYLAND_DISPLAY=//p'); "
            + command
        )
    )


def plug(g, on):
    g.shell(
        f"echo {'on' if on else 'off'} >/sys/class/drm/card0-Virtual-2/status && echo change >/sys/class/drm/card0/uevent"
    )


def assert_session(g):
    assert g.qmp("query-display-options")["type"] == "none"
    nodes = {n["node-name"]: n["ro"] for n in g.qmp("query-named-block-nodes")}
    assert nodes["base"] and nodes["basefile"] and not nodes["layer"]
    assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
    return int(g.shell("pgrep -x -u joe mundane-shell"))


def bar_presented(g, head, width, height, scale):
    path = g.run / f"bar-output-{head}.ppm"
    g.qmp("screendump", device="gpu", head=head, filename=str(path))
    header = f"P6\n{width} {height}\n255\n".encode()
    raw = path.read_bytes()
    if not raw.startswith(header):
        return False
    data = memoryview(raw)[len(header) :]

    def pixel(x, y):
        n = 3 * (y * width + x)
        return bytes(data[n : n + 3])

    # The bar fills the scanout at its bottom margin. Wallpaper is visible above it.
    y = height - 2 * scale
    base = pixel(width // 2, y)
    return (
        pixel(2 * scale, y) == base == pixel(width - 2 * scale, y)
        and pixel(width // 2, height - 45 * scale) != base
    )


def qualify(run, probe):
    g = Guest(run)
    pid = assert_session(g)
    shutil.copyfile(probe, g.run / "share/nested-capabilities")
    g.shell(
        "mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && install -m755 /mnt/nested-capabilities /tmp/gpui-client && umount /mnt"
    )
    joe(
        g,
        "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true",
    )

    def bars():
        return (
            joe(g, "python3 -W ignore /usr/local/libexec/mundane-test-widgets list")
            .splitlines()
            .count("frame Taskbar")
        )

    def outputs():
        return json.loads(joe(g, "mundane-de output list"))

    def area(name, w, h, cursor=None):
        after = " --after-cursor=" + shlex.quote(cursor) if cursor else ""
        log = g.shell(
            "journalctl -b -o cat --no-pager --grep='layer commit'"
            + after
            + " | tail -60"
        )
        log = re.sub(r"\x1b\[[0-9;]*m", "", log)
        return f'role="mundane-taskbar" width={w} height={h}' in "\n".join(
            line for line in log.splitlines() if f'output="{name}"' in line
        )

    work = joe(g, "mktemp -d")
    client = int(
        joe(
            g,
            f'/tmp/gpui-client "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" --shell {work} >{work}/log 2>&1 </dev/null & echo $!',
        )
    )
    try:
        wait(
            "ordinary and sandboxed clients lack privileged globals",
            lambda: (
                "ordinary and sandbox registries passed" in joe(g, f"cat {work}/log")
            ),
        )
        wait(
            "primary reserves 44 logical pixels",
            lambda: bars() == 1 and area("Virtual-1", 1920, 1036),
        )
        for _ in range(2):
            plug(g, True)
            wait("separate native bar on each output", lambda: bars() == 2)
            secondary = next(o for o in outputs() if o["name"] == "Virtual-2")
            m = secondary["current"]
            w, h = m["width"], m["height"]
            cursor = g.shell("journalctl -b -n0 --show-cursor").split("-- cursor: ")[1]
            joe(
                g,
                f"mundane-de output test Virtual-2 {w} {h} {round(m['refresh_hz'] * 1000)} 2",
            )
            try:
                wait(
                    "scaled work area",
                    lambda w=w, h=h, cursor=cursor: area(
                        "Virtual-2", w // 2, h // 2 - 44, cursor
                    ),
                )
                assert (
                    next(o for o in outputs() if o["name"] == "Virtual-1")["scale"] == 1
                )
                assert bars() == 2

                wait(
                    "presented bars span both physical scanouts",
                    lambda w=w, h=h: (
                        bar_presented(g, 0, 1920, 1080, 1)
                        and bar_presented(g, 1, w, h, 2)
                    ),
                )
            finally:
                joe(g, "mundane-de output revert")
            plug(g, False)
            wait("one surviving bar", lambda: bars() == 1 and len(outputs()) == 1)
            assert assert_session(g) == pid
        print(
            "Distinct 1× and 2× work areas, two reconnects and unprivileged registries passed."
        )
    finally:
        plug(g, False)
        g.shell(f"kill {client} 2>/dev/null || :")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument(
        "--probe",
        type=Path,
        default=Path("target/release/examples/nested-capabilities"),
    )
    a = p.parse_args()
    qualify(a.run, a.probe)
