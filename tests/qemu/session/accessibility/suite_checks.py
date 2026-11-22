"""Exercise the suite's shipped Terminal and GPUI's motion preview on the guest seat."""

import json
import time


def drag_from_centre(g, x, y):
    # Use only this guest's absolute tablet and keyboard.
    g.move(0.5, 0.5)

    def event(kind, **data):
        g.qmp("input-send-event", events=[dict(type=kind, data=data)])

    alt = dict(type="qcode", data="alt")
    event("key", key=alt, down=True)
    event("btn", button="left", down=True)
    g.move(x, y)
    event("btn", button="left", down=False)
    event("key", key=alt, down=False)
    time.sleep(0.25)


def check(g, joe, launch, activate, tree, read, wait):
    # Start over the open Settings window. AT-SPI actions on Settings do not
    # raise it. Only sample Terminal's grid, clear of its header and shell chrome.
    def frame():
        magic, size, depth, pixels = g.screen("suite-appearance").split(b"\n", 3)
        assert (magic, size, depth) == (b"P6", b"1920 1080", b"255")
        return pixels

    def region(pixels, x, y, width, height):
        return b"".join(
            pixels[3 * (row * 1920 + x) : 3 * (row * 1920 + x + width)]
            for row in range(y, y + height)
        )

    def palette(terminal, taskbar):
        pixels = frame()
        grid = region(pixels, 1640, 510, 180, 60)
        bar = region(pixels, 620, 1054, 120, 12)
        return (
            grid.count(bytes.fromhex(terminal)) > 9000
            and bar.count(bytes.fromhex(taskbar)) > 700
        )

    def native(key, value):
        return json.loads(read("gtk-style"))[key] == value

    terminal = launch("/usr/local/bin/mundane-terminal", "suite-terminal")
    try:
        wait("mapped suite Terminal", lambda: "frame Terminal" in tree())
        drag_from_centre(g, 0.76, 0.5)
        wait("initial dark Terminal grid", lambda: palette("17201e", "1b2130"))
        activate("Dark appearance")
        wait(
            "light native app and Terminal",
            lambda: native("dark", False) and palette("f8faf8", "fafafa"),
        )
        activate("Dark appearance")
        wait(
            "dark native app and Terminal",
            lambda: native("dark", True) and palette("17201e", "1b2130"),
        )
        activate("High contrast")
        wait(
            "high contrast in native app and Terminal",
            lambda: native("contrast", True) and palette("000000", "000000"),
        )
        activate("High contrast")
        wait(
            "regular contrast restored",
            lambda: native("contrast", False) and palette("17201e", "1b2130"),
        )
        # The per-desktop schema default applies to mundaneDE after a reset.
        joe("gsettings reset org.gnome.desktop.interface color-scheme")
        wait(
            "default GTK and suite scheme",
            lambda: native("dark", False) and palette("f8faf8", "fafafa"),
        )
        activate("Dark appearance")
        wait(
            "dark restored",
            lambda: native("dark", True) and palette("17201e", "1b2130"),
        )
        # The focused terminal cursor uses the shared theme's accent.
        activate("Accent colour")
        wait("new native app accent", lambda: native("accent", "teal"))
        wait(
            "accent on Terminal cursor",
            lambda: (
                region(frame(), 1020, 220, 835, 650).count(bytes.fromhex("2190a4"))
                >= 50
            ),
        )
    finally:
        g.shell(f"kill {terminal} 2>/dev/null || :")
    motion = launch("/tmp/m7/suite-motion", "suite-motion")
    try:
        wait("motion preview mapped", lambda: "Motion preview" in tree())

        def marker():
            return region(frame(), 740, 420, 440, 240)

        initial = marker()
        for delay in (0.17, 0.23):
            time.sleep(delay)
            assert marker() == initial, "reduced motion did not hold the preview still"
        activate("Reduce motion")
        wait("native animations enabled", lambda: native("animations", True))
        initial = marker()
        wait("GPUI marker moves", lambda: marker() != initial)
        time.sleep(0.25)
        moving = marker()
        wait("GPUI marker continues moving", lambda: marker() != moving)
        activate("Reduce motion")
        wait("native animations disabled", lambda: native("animations", False))
        time.sleep(0.3)
        stopped = marker()
        for delay in (0.19, 0.29, 0.41):
            time.sleep(delay)
            assert marker() == stopped, "marker continued moving"
    finally:
        g.shell(f"kill {motion} 2>/dev/null || :")
    print(
        "A single Settings action updated libadwaita and the open suite Terminal for scheme, contrast and accent. GPUI motion stopped and resumed with GTK animations.",
        flush=True,
    )
