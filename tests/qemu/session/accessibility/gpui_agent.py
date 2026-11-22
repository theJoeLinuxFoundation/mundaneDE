"""Exercise the running GPUI shell with a private authority in the test guest."""

import shlex
import time


def qualify(g, joe, launch, tree, speech, wait):
    reader = "pgrep -u joe -f '^/usr/bin/python3 /usr/bin/orca' || :"
    before = g.shell(reader)
    assert before.isdigit()
    offset = len(speech())
    g.shell("pkill -TERM -x mundane-shell")
    wait(
        "replacement supervised reader",
        lambda: (pid := g.shell(reader)).isdigit() and pid != before,
    )
    wait(
        "reader initialized after shell restart",
        lambda: "Screen reader on." in speech()[offset:],
    )
    offset = len(speech())
    joe("gsettings set org.gnome.desktop.notifications show-banners true")
    joe(
        """gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications --method org.freedesktop.Notifications.Notify Files 0 '' 'Download complete' 'Report is ready' '[]' '{}' 5000"""
    )
    wait(
        "Orca spoke the delivered notification",
        lambda: any("Download complete" in s for s in speech()[offset:]),
    )
    print(
        "Shell restart replaced Orca. Orca spoke the delivered notification.",
        flush=True,
    )

    directory = joe("mktemp -d /tmp/m7/authority.XXXXXX")
    address = "unix:path=" + directory + "/bus"
    daemon = int(joe(f"dbus-daemon --session --fork --address={address} --print-pid"))
    sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")

    def authority():
        return launch(
            f"env DBUS_SESSION_BUS_ADDRESS={address} python3 /tmp/m7/gpui_authority.py {directory} {sid}",
            "authority",
        )

    fixture = authority()
    binary = "/usr/local/bin/mundane-shell"
    real = binary + ".fixture-real"
    wrapper = f'#!/bin/sh\nexport DBUS_SYSTEM_BUS_ADDRESS={address}\nexec {real} "$@"\n'
    widgets = "python3 -W ignore /tmp/m7/shell_widgets.py click "

    def command(method, cookie):
        joe(
            f"echo {method} {cookie} > {directory}/next && mv {directory}/next {directory}/command"
        )

    def audit(line):
        return line in joe(f"cat {directory}/audit 2>/dev/null || :").splitlines()

    def prompt():
        return "password text Password" in tree()

    try:
        g.shell(
            f"test ! -e {real} && mv {binary} {real} && printf %s {shlex.quote(wrapper)} > {binary} && chmod 755 {binary} && pkill -x mundane-shell"
        )
        wait(
            "agent registered for the active session",
            lambda: audit("RegisterAuthenticationAgent"),
        )
        dest, path = joe(f"cat {directory}/target").split()
        denied = joe(
            f"gdbus call --address {address} --dest {dest} --object-path {path} --method org.freedesktop.PolicyKit1.AuthenticationAgent.CancelAuthentication trial 2>&1 || :"
        )
        assert "AccessDenied" in denied, denied
        command("BeginAuthentication", "chooser")
        wait("two-identity helper conversation", prompt)
        assert "org.joelinux.test" not in tree()
        g.click(0.02, 0.98)
        assert prompt() and "frame Start menu" not in tree()
        joe(widgets + "Details")
        wait(
            "action and details",
            lambda: (
                "org.joelinux.test" in tree() and "device: Virtual display" in tree()
            ),
        )
        assert "Administrator: root" in tree()
        joe(widgets + "Details")
        joe(widgets + "Change")
        option = next(
            row.split(" ", 2)[-1]
            for row in tree().splitlines()
            if row.startswith("menu item ") and row.endswith((" joe", "(joe)"))
        )
        joe(widgets + shlex.quote(option))
        wait(
            "new conversation for joe",
            lambda: "Administrator: joe" in tree() and prompt(),
        )
        for appearance in ("light", "dark", "contrast"):
            scheme = "prefer-light" if appearance == "light" else "prefer-dark"
            joe("gsettings set org.gnome.desktop.interface color-scheme " + scheme)
            joe(
                "gsettings set org.gnome.desktop.a11y.interface high-contrast "
                + str(appearance == "contrast").lower()
            )
            color = {"light": "e9eeea", "dark": "222e2a", "contrast": "000000"}[
                appearance
            ]
            wait(
                "authentication appearance " + appearance,
                lambda appearance=appearance, color=color: (
                    g.screen("gpui-administrator-" + appearance)
                    .split(b"\n", 3)[3]
                    .count(bytes.fromhex(color))
                    > 20000
                ),
            )
        joe("gsettings set org.gnome.desktop.a11y.interface high-contrast false")

        # Track the long horizontal focus border as the denied password field
        # reappears. Both the motion and the stationary retry use real scanout.
        def field_x():
            pixels = g.screen("polkit-retry").split(b"\n", 3)[3]
            border = bytes.fromhex("2190a4") * 300  # suite's teal accent
            edge = pixels.find(border, 320 * 1920 * 3, 800 * 1920 * 3)
            return edge // 3 % 1920 if edge >= 0 else None

        for animations in (True, False):
            joe(
                "gsettings set org.gnome.desktop.interface enable-animations "
                + str(animations).lower()
            )
            time.sleep(0.3)
            baseline = field_x()
            assert baseline is not None
            g.key("x")
            time.sleep(0.12)
            g.key("ret")
            deadline = time.monotonic() + 8
            pending = False
            positions = []
            while time.monotonic() < deadline:
                x = field_x()
                if x is None:
                    pending = True
                elif pending:
                    if not positions:
                        deadline = time.monotonic() + 0.45
                    positions.append(x)
            assert pending and positions and positions[-1] == baseline, (
                baseline,
                positions,
            )
            assert (max(positions) != min(positions)) == animations, positions
            assert "Authentication failed. Try again." in tree()
        print(
            "Password retry moved with animations enabled and stayed still with reduced motion.",
            flush=True,
        )
        command("CancelAuthentication", "stale")
        wait("old cookie returned", lambda: audit("CancelAuthentication stale ok"))
        assert prompt()
        command("CancelAuthentication", "chooser")
        wait(
            "authority cancellation",
            lambda: (
                audit(
                    "BeginAuthentication chooser org.freedesktop.PolicyKit1.Error.Cancelled"
                )
                and not prompt()
            ),
        )
        g.key("meta_l", "l")
        wait(
            "session locked",
            lambda: (
                g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes"
            ),
        )
        command("BeginAuthentication", "locked")
        wait(
            "request rejected while locked",
            lambda: audit(
                "BeginAuthentication locked org.freedesktop.PolicyKit1.Error.Cancelled"
            ),
        )
        assert "frame Authentication required" not in tree()
        wait(
            "protected locker field",
            lambda: "frame Session locked" in tree() and prompt(),
        )
        for key in ("j", "o", "e", "ret"):
            g.key(key)
            time.sleep(0.14)
        wait(
            "session unlocked",
            lambda: (
                g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "no"
            ),
        )
        command("BeginAuthentication", "vanish")
        wait("second prompt", prompt)
        g.shell(f"kill {fixture}")
        wait("authority disappeared", lambda: not prompt())
        joe(f"rm {directory}/target")
        fixture = authority()
        wait(
            "agent registered with replacement authority",
            lambda: joe(f"cat {directory}/target 2>/dev/null || :"),
        )
        g.shell(f"mv {real} {binary}")
        g.key("meta_l", "s")
        wait("system menu", lambda: "button Log Out" in tree())
        joe(widgets + "'Log Out'")
        wait(
            "orderly agent unregistration",
            lambda: audit("UnregisterAuthenticationAgent"),
        )
        print(
            "Private polkit authority, two administrators, details, caller restriction, lock refusal, stale cookie, authority loss and logout unregistration passed.",
            flush=True,
        )
    finally:
        g.shell(
            f"if test -e {real}; then mv -f {real} {binary}; fi; systemctl restart gdm"
        )
        g.shell(f"kill {daemon} {fixture} 2>/dev/null || :")
        time.sleep(2)
