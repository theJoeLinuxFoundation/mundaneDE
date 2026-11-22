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


def qualify(run):
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
        )

    # Reuse the untouched system font caches from the image. Give GPUI/Mesa a
    # fresh user cache for the cold measurement and keep it for the warm logins.
    cache = joe(g, "mktemp -d /home/joe/.cache/gpui-qualification.XXXXXX")
    entry = "/usr/share/wayland-sessions/mundane-gpui.desktop"
    g.shell(
        f"sed -i -E 's! XDG_CACHE_HOME=[^ ]+!!;s!^Exec=/usr/bin/env !Exec=/usr/bin/env XDG_CACHE_HOME={cache} !' {entry}"
    )
    samples = []
    for i in range(21):
        g.shell("systemctl stop gdm")
        after = clock()
        g.shell("systemctl reset-failed gdm; systemctl start gdm")
        wait("new shell launch", lambda after=after: stamp("launch shell") > after)
        start = stamp("launch shell")
        wait(
            "new bar submitted",
            lambda start=start: stamp("layer commit.*mundane-taskbar") >= start,
        )
        wait("presented Start button", bar)
        # QEMU suppresses unchanged tablet axes across GDM seat reopen.
        g.move(0.48, 0.42)
        g.click(32 / 1920, 1056 / 1080)
        wait("Start presented after seat click", menu)
        elapsed = clock() - start
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
    assert sorted(samples)[18] <= 2
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
        orca = joe(g, "orca --replace </dev/null >/tmp/gpui-orca.log 2>&1 & echo $!")
        try:
            idle("Two outputs with Orca")
            print(
                "Orca memory:",
                g.shell(f"grep -E '^(Pss|Rss):' /proc/{int(orca)}/smaps_rollup"),
                flush=True,
            )
        finally:
            g.shell(f"kill {int(orca)} 2>/dev/null || :")
    finally:
        plug(g, False)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    qualify(parser.parse_args().run)
