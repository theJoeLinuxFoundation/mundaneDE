"""Drive the active GDM login using real Settings controls and the virtual keyboard."""

import json
import shlex
import time


def check(g, joe, launch, tree, speech, read, wait):
    widgets = "python3 -W ignore /tmp/m7/shell_widgets.py "

    def activate(label):
        wait(
            label, lambda: any(row.endswith(" " + label) for row in tree().splitlines())
        )
        joe(widgets + "click " + shlex.quote(label))

    def pixel(label, color):
        return g.screen(label).split(b"\n", 3)[3].count(bytes.fromhex(color)) > 20000

    def pref(schema, key, value):
        joe(f"gsettings set org.gnome.desktop.{schema} {key} {value}")

    def state(label):
        return joe(widgets + "state " + shlex.quote(label)).split()

    offset = len(speech())
    g.key("meta_l", "spc")
    wait(
        "launcher speech",
        lambda: any("Search applications" in s for s in speech()[offset:]),
    )
    for key in ("t", "e", "x", "t", "spc", "e", "d", "i", "t", "o", "r"):
        g.key(key)
        time.sleep(0.12)
    time.sleep(0.3)
    g.key("ret")
    wait(
        "editor launched with keyboard",
        lambda: (
            "application gnome-text-editor" in tree()
            and "frame Start menu" not in tree()
        ),
    )
    g.key("meta_l", "q")

    # An existing app and a newly mapped Settings use the same live preferences.
    native = launch("python3 /tmp/m7/style.py", "style")
    try:
        wait("native GTK", lambda: read("gtk-style"))
        pref("a11y.interface", "high-contrast", "false")
        pref("interface", "enable-animations", "true")
        pref("interface", "color-scheme", "prefer-light")
        g.key("meta_l", "comma")
        activate("Appearance")
        wait("light GPUI paint", lambda: pixel("settings-light", "f9faf8"))
        activate("Dark appearance")
        wait("dark GPUI paint", lambda: pixel("settings-dark", "17201e"))
        wait("native dark", lambda: json.loads(read("gtk-style"))["dark"])
        # Navigate into the toggle with a real Tab key; inspect the role and
        # state spoken by Orca, then use Space to activate that same control.
        joe(widgets + "focus 'Dark appearance'")
        offset = len(speech())
        g.key("tab")
        wait(
            "contrast focus speech",
            lambda: any("High contrast toggle button" in s for s in speech()[offset:]),
        )
        assert "focused" in state("High contrast")
        offset = len(speech())
        g.key("spc")
        wait("Orca announces toggle state", lambda: "pressed" in speech()[offset:])
        wait("GPUI pressed", lambda: "pressed" in state("High contrast"))
        wait("native contrast", lambda: json.loads(read("gtk-style"))["contrast"])
        wait("GPUI contrast paint", lambda: pixel("settings-contrast", "000000"))
        activate("Reduce motion")
        wait(
            "native reduced motion",
            lambda: not json.loads(read("gtk-style"))["animations"],
        )
        assert "pressed" in state("Reduce motion")
        activate("High contrast")
        pref("interface", "accent-color", "purple")
        wait(
            "native accent", lambda: json.loads(read("gtk-style"))["accent"] == "purple"
        )
        wait(
            "GPUI accent paint",
            lambda: (
                g.screen("settings-accent")
                .split(b"\n", 3)[3]
                .count(bytes.fromhex("9141ac"))
                > 100
            ),
        )
        # Exercise the same live window across actual DRM fractional-scale changes.
        output = json.loads(joe("/usr/local/bin/mundane-de output list"))[0]
        mode = output["current"]
        command = shlex.join(
            [
                "/usr/local/bin/mundane-de",
                "output",
                "test",
                output["name"],
                str(mode["width"]),
                str(mode["height"]),
                str(round(mode["refresh_hz"] * 1000)),
            ]
        )
        for factor in (1.0, 1.5, 2.0):
            joe(command + " " + str(factor))
            try:
                wait(
                    "output scale",
                    lambda: (
                        json.loads(joe("/usr/local/bin/mundane-de output list"))[0][
                            "scale"
                        ]
                        == factor
                    ),
                )
                wait(
                    "scaled GPUI frame",
                    lambda: pixel("settings-output-" + str(factor), "17201e"),
                )
                assert "toggle button Reduce motion" in tree()
            finally:
                joe("/usr/local/bin/mundane-de output revert")
        for scale in ("1.0", "1.5", "2.0"):
            pref("interface", "text-scaling-factor", scale)
            time.sleep(0.25)
            assert "toggle button Reduce motion" in tree()
            g.screen("settings-text-" + scale)
        pref("interface", "text-scaling-factor", "1.0")
        activate("Display")
        wait("connected display mode", lambda: "button Mode:" in tree())
        before = json.loads(joe("/usr/local/bin/mundane-de output list"))[0]["scale"]
        scale = next(
            row.removeprefix("button ")
            for row in tree().splitlines()
            if row.startswith("button Scale:")
        )
        activate(scale)
        activate("Test layout")
        wait(
            "output trial",
            lambda: (
                json.loads(joe("/usr/local/bin/mundane-de output list"))[0]["scale"]
                != before
            ),
        )
        activate("Revert layout")
        wait(
            "previous layout",
            lambda: (
                json.loads(joe("/usr/local/bin/mundane-de output list"))[0]["scale"]
                == before
            ),
        )
        activate("Date & Time")
        original = g.shell("timedatectl show -p Timezone --value")
        target = "Africa/Cairo" if original != "Africa/Cairo" else "Africa/Abidjan"

        def choose():
            activate("Timezone")
            activate(target)
            wait("shell authentication", lambda: "password text Password" in tree())

        choose()
        activate("Cancel")
        wait("cancelled", lambda: "frame Authentication required" not in tree())
        assert g.shell("timedatectl show -p Timezone --value") == original
        choose()
        g.shell("pkill -TERM -x pkexec")
        wait(
            "authority withdrew the dead requester",
            lambda: "frame Authentication required" not in tree(),
        )
        choose()
        g.key("meta_l", "l")
        sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
        wait(
            "locked",
            lambda: (
                g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes"
            ),
        )
        assert "frame Authentication required" not in tree()

        def password():
            for key in ("j", "o", "e", "ret"):
                g.key(key)
                time.sleep(0.14)

        password()
        wait(
            "unlocked",
            lambda: (
                g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "no"
            ),
        )
        assert g.shell("timedatectl show -p Timezone --value") == original
        choose()
        offset = len(speech())
        password()
        wait(
            "timezone changed",
            lambda: g.shell("timedatectl show -p Timezone --value") == target,
        )
        assert not set(speech()[offset:]) & {"j", "o", "e", "joe"}
        assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
        g.shell("timedatectl set-timezone " + shlex.quote(original))
        print(
            "Launcher and Settings speech, live native and GPUI appearance, display rollback, real polkit cancellation, lock and PAM authorization passed.",
            flush=True,
        )
        g.key("meta_l", "q")
    finally:
        g.shell(f"kill {native} 2>/dev/null || :")
