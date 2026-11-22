"""Measure the GPUI shell on a trace-enabled private joe guest. Run on a fresh overlay."""

import argparse
import statistics
import time
from pathlib import Path

from guest import Guest
from qualify_gpui_outputs import assert_preview, joe, plug
from qualify_menus import wait


def pixel(frame, x, y):
    start = (1920 * y + x) * 3
    return frame[start : start + 3]


def qualify(run, startup_only=False):
    g = Guest(run)
    assert_preview(g)

    def journal(pattern):
        return g.shell(f"journalctl -b -o short-unix --no-pager --grep='{pattern}' -n1")

    def stamp(pattern):
        return float(journal(pattern).split()[0])

    def clock():
        return float(g.shell("date +%s.%N"))

    def frame():
        data = g.screen("budget")
        return data.split(b"\n", 3)[-1] if data.startswith(b"P6\n1920 1080\n") else b""

    def bar():
        p = frame()
        return (
            p
            and pixel(p, 8, 1054) != pixel(p, 500, 1054)
            and pixel(p, 500, 1054) == pixel(p, 700, 1041)
        )

    def menu():
        p = frame()
        return (
            p
            and pixel(p, 19, 515) == pixel(p, 500, 1054)
            and pixel(p, 600, 515) != pixel(p, 19, 515)
            # Count a populated pinned grid, even on the first login.
            and len(
                {pixel(p, x, y) for y in range(591, 610, 3) for x in range(18, 428, 2)}
            )
            > 20
        )

    # Reuse the untouched system font caches from the image. Give GPUI/Mesa a
    # fresh user cache for the cold measurement and keep it for the warm logins.
    cache = joe(g, "mktemp -d /home/joe/.cache/gpui-qualification.XXXXXX")
    entry = "/usr/share/wayland-sessions/mundane-gpui.desktop"
    g.shell(
        f"sed -i -E 's! XDG_CACHE_HOME=[^ ]+!!;s!^Exec=/usr/bin/env !Exec=/usr/bin/env XDG_CACHE_HOME={cache} !' {entry}"
    )

    def presented(label, matches):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if matches():
                return
            time.sleep(0.008)
        raise AssertionError(label)

    samples = []
    for i in range(21):
        g.shell("systemctl stop gdm")
        after = clock()
        g.shell("systemctl reset-failed gdm; systemctl start gdm")
        # Observe actual scanout. Journal scans and 120 ms sleeps in the timed
        # path used to delay the seat click and quantize the result.
        presented("presented Start button", bar)
        # QEMU suppresses unchanged tablet axes across GDM seat reopen.
        g.move(0.48, 0.42)
        g.click(32 / 1920, 1056 / 1080)
        presented("Start presented after seat click", menu)
        end = clock()
        start = stamp("launch shell")
        assert after <= start < end, "stale login"
        elapsed = end - start
        records = g.shell(
            f"journalctl -b --since=@{start:.6f} -o short-unix --no-pager -g 'layer commit.*mundane-(taskbar|start)'"
        )

        def first(role):
            return next(
                float(row.split()[0])
                for row in records.splitlines()
                if f"mundane-{role}" in row
            )

        print(
            f"First bar submission {first('taskbar') - start:.3f} s, first Start submission {first('start') - start:.3f} s",
            flush=True,
        )
        joe(
            g,
            "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true",
        )
        wait(
            "search has keyboard focus",
            lambda: (
                "focused"
                in joe(
                    g,
                    "python3 -W ignore /usr/local/libexec/mundane-test-widgets state 'Search applications' || :",
                ).split()
            ),
        )
        if i == 0:
            print(f"Cold user cache: {elapsed:.3f} s", flush=True)
            continue
        samples.append(elapsed)
        print(f"Login {i}: {elapsed:.3f} s", flush=True)
    print(
        f"Usable Start, median {statistics.median(samples):.3f} s, p95 {sorted(samples)[18]:.3f} s",
        flush=True,
    )
    assert sorted(samples)[18] < 0.5, "usable Start p95 must be below 0.5 seconds"
    if startup_only:
        return
    g.key("esc")
    g.move(0.48, 0.38)

    def idle(label):
        time.sleep(60)
        pid = assert_preview(g)

        def cpu():
            fields = g.shell(f"cat /proc/{pid}/stat").split()
            return int(fields[13]) + int(fields[14])

        begin = cpu()
        start = time.monotonic()
        time.sleep(60)
        percent = (
            100
            * (cpu() - begin)
            / (time.monotonic() - start)
            / int(g.shell("getconf CLK_TCK"))
        )
        memory = g.shell(f"grep -E '^(Pss|Rss):' /proc/{pid}/smaps_rollup")
        print(f"{label}: {percent:.3f}% of one vCPU; {memory}", flush=True)
        peers = g.shell("""for pid in $(ps -u joe -o pid=); do
            exe=$(readlink "/proc/$pid/exe") || continue
            case "${exe##*/}" in
              mundane-de|pipewire|wireplumber|xdg-desktop-portal|xdg-desktop-portal-gtk)
                awk -v name="${exe##*/}" '/^Pss:/ { print name, $2, "KiB" }' "/proc/$pid/smaps_rollup";;
            esac
        done""")
        print("Session peers:", peers, flush=True)
        assert percent <= 0.5
        if label == "One output":
            pss = int(
                next(
                    line for line in memory.splitlines() if line.startswith("Pss:")
                ).split()[1]
            )
            assert pss <= 150 * 1024
            print(
                "Renderer:",
                g.shell(
                    f"awk '/libvulkan_.*so/ {{print $NF}}' /proc/{pid}/maps | sort -u"
                ),
                flush=True,
            )

    def clock_pixels():
        p = frame()
        return b"".join(
            p[(y * 1920 + 1850) * 3 : (y * 1920 + 1916) * 3] for y in range(1045, 1075)
        )

    def quiet():
        # Sample between minute boundaries, verify the clock then makes a new commit.
        if int(g.shell("date +%S")) > 45:
            time.sleep(16)
        before_clock = clock_pixels()
        last = journal("DRM redraw|layer commit")
        time.sleep(10)
        assert journal("DRM redraw|layer commit") == last, (
            "redraw or layer commit while idle"
        )
        wait(
            "clock submits its next update",
            lambda: journal("DRM redraw|layer commit") != last,
            65,
        )
        wait("clock changes on scanout", lambda: clock_pixels() != before_clock)
        time.sleep(0.25)
        last = journal("DRM redraw|layer commit")
        time.sleep(10)
        assert journal("DRM redraw|layer commit") == last
        print(
            "No shell commits or DRM redraws in both 10-second idle windows; clock updated.",
            flush=True,
        )

    idle("One output")
    quiet()
    joe(g, "gsettings set org.gnome.desktop.interface enable-animations false")
    time.sleep(0.5)
    quiet()
    joe(g, "gsettings reset org.gnome.desktop.interface enable-animations")
    plug(g, True)
    try:
        idle("Two outputs")
        setting = "org.gnome.desktop.a11y.applications screen-reader-enabled"
        previous = joe(g, "gsettings get " + setting)
        joe(g, "gsettings set " + setting + " true")
        reader = "pgrep -P " + str(assert_preview(g)) + " -f /usr/bin/orca || :"
        wait("supervised Orca", lambda: g.shell(reader).isdigit())
        orca = int(g.shell(reader))
        try:
            idle("Two outputs with Orca")
            print(
                "Orca memory:",
                g.shell(f"grep -E '^(Pss|Rss):' /proc/{int(orca)}/smaps_rollup"),
                flush=True,
            )
        finally:
            joe(g, f"gsettings set {setting} {previous}")
    finally:
        plug(g, False)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--startup-only", action="store_true")
    args = parser.parse_args()
    qualify(args.run, args.startup_only)
