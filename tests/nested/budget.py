"""Measure the existing nested renderer under client placement, drags, snaps and wallpaper fades."""

import json
import math
from pathlib import Path
import re
import subprocess
import sys
import time
from run import OUTPUT, run, wait_for

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def samples(path):
    lines = ANSI.sub("", path.read_text(errors="replace"))
    return [
        tuple(map(int, m))
        for m in re.findall(
            r"submitted frame=\d+ spring_us=(\d+) cpu_us=(\d+) submit_us=(\d+) time_us=(\d+)", lines
        )
    ]


def percentiles(ns):
    ordered = sorted(ns)
    return [round(ordered[math.ceil(len(ns) * q) - 1], 3) for q in (0.50, 0.95, 0.99)]


def measure(base, binary, inner, host, launch, swaymsg, hz):
    assert swaymsg(f"output {OUTPUT} mode --custom 1920x1080@{hz}Hz")[0]["success"]
    mode = swaymsg("-t", "get_outputs")[0]["current_mode"]
    assert (mode["width"], mode["height"], mode["refresh"]) == (1920, 1080, hz * 1000), mode
    log = base / "mundane.log"
    client = launch(
        [sys.executable, str(Path(__file__).with_name("frame_scene.py"))],
        base / "scene.log",
        dict(inner, GDK_BACKEND="wayland", GSK_RENDERER="cairo", GTK_A11Y="none"),
        stdin=subprocess.PIPE,
    )
    wait_for("ten mapped windows", lambda: log.read_text().count("mapped window") >= 11, [client])
    driver = binary.parent / "examples/nested-input"
    socket = Path(host["XDG_RUNTIME_DIR"]) / host["WAYLAND_DISPLAY"]
    keys = base / "keymap"
    keys.write_bytes(run(["xkbcli", "compile-keymap"], host))

    def input(*args):
        run([driver, socket, *map(str, args)], host)

    time.sleep(3)
    start = len(samples(log))
    end = time.monotonic() + 60
    while time.monotonic() < end:
        input("pointer", "423,250", 272, "360,210", "320,190", -272)
        input(keys, 125, 17, -17, -125)
        time.sleep(min(3, max(0, end - time.monotonic())))
    frames = samples(log)[start:]
    client.stdin.write(b"idle\n")
    client.stdin.flush()
    time.sleep(3)
    idle = len(samples(log))
    redraws = log.read_text().count("repaint")
    time.sleep(10)
    redraws = log.read_text().count("repaint") - redraws
    idle = len(samples(log)) - idle
    assert log.read_text().count("wallpaper reload") >= 2
    assert log.read_text().count("released window") >= 2
    assert len(frames) > hz * 45, (len(frames), hz)
    intervals = [(b[3] - a[3]) / 1000 for a, b in zip(frames, frames[1:])]
    missed = sum(max(0, round(ms * hz / 1000) - 1) for ms in intervals)
    report = dict(
        hz=hz,
        mode=mode,
        frames=len(frames),
        measured_s=(frames[-1][3] - frames[0][3]) / 1e6,
        spring_ms=percentiles([r[0] / 1000 for r in frames]),
        update_submit_cpu_ms=percentiles([r[1] / 1000 for r in frames]),
        update_submit_ms=percentiles([r[2] / 1000 for r in frames]),
        compose_submit_cpu_ms=percentiles([(r[1] - r[0]) / 1000 for r in frames]),
        submission_interval_ms=percentiles(intervals),
        missed_percent=round(100 * missed / (len(frames) + missed), 3),
        idle_submissions=idle,
        idle_repaints=redraws,
    )
    (base / "budget.json").write_text(json.dumps(report, indent=2) + "\n")
    print(report, flush=True)
    assert report["spring_ms"][1] < 1
    assert report["update_submit_cpu_ms"][1] < (8 if hz == 60 else 4)
    assert report["missed_percent"] < 1 and idle == 0 and redraws == 0
