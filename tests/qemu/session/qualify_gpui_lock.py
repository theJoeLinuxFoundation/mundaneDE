"""Exercise the GPUI lock on the virtual seat of a dedicated enforcing joe guest."""

import argparse
import functools
import json
import subprocess
import threading
import time
from pathlib import Path

from qualify_gpui_settings import FIX, Session
from qualify_menus import wait


def qualify(run, output):
    s = Session(run, output)
    g = s.g
    sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
    hint = f"loginctl show-session {sid} -p LockedHint --value"
    exe = "/usr/local/bin/mundane-lock"
    held = exe + ".qualification"
    pam_service = "/usr/lib/pam.d/mundane-lock"
    original_service = pam_service + ".qualification"
    assert (
        g.shell(
            f"test -x {exe} && test ! -e {held} && test ! -e {original_service} && echo yes"
        )
        == "yes"
    )
    settings = {
        "org.gnome.desktop.interface color-scheme": "prefer-dark",
        "org.gnome.desktop.interface enable-animations": "true",
        "org.gnome.desktop.a11y.interface high-contrast": "false",
        "org.gnome.desktop.a11y.applications screen-reader-enabled": "true",
    }
    old = {k: s.joe("gsettings get " + k) for k in settings}
    g.shell(
        f"install -d -o joe -g users /home/joe/.local/share/orca && cp {FIX}/recording_speech.py {FIX}/orca-customizations.py /home/joe/.local/share/orca/"
    )
    for k, v in settings.items():
        s.joe(f"gsettings set {k} {v}")

    def speech():
        return [
            json.loads(row)[1] for row in s.read("mundane-speech.jsonl").splitlines()
        ]

    def pid():
        return g.shell("pgrep -x -u joe mundane-lock || :")

    def plug(n, on):
        g.shell(
            f"echo {'on' if on else 'off'} >/sys/class/drm/card0-Virtual-{n}/status && echo change >/sys/class/drm/card0/uevent"
        )

    def shot(n):
        dest = g.run / f"gpui-lock-scanout-{threading.current_thread().ident}-{n}.ppm"
        g.qmp("screendump", device="gpu", head=n, filename=str(dest))
        fmt, dims, depth, pixels = dest.read_bytes().split(b"\n", 3)
        assert fmt == b"P6" and depth == b"255"
        return tuple(map(int, dims.split())), pixels

    mask = bytes.fromhex("0a121a")
    desktop = bytes.fromhex("2088df")

    @functools.cache
    def wallpaper(width, height):
        # Independent reference from the installed default artwork, outside the
        # compositor. Bilinear texture sampling and JPEG decoders can round differently.
        return subprocess.check_output(
            [
                "magick",
                str(Path(__file__).resolve().parents[3] / "assets/afterlight.jpg"),
                "-filter",
                "triangle",
                "-resize",
                f"{width}x{height}^",
                "-gravity",
                "center",
                "-extent",
                f"{width}x{height}",
                "-evaluate",
                "multiply",
                "0.46",
                "-depth",
                "8",
                "rgb:-",
            ]
        )

    def looks_locked(frame):
        (w, h), p = frame
        return (
            p[:3] == mask
            or max(abs(a - b) for a, b in zip(p[:3], wallpaper(w, h)[:3])) <= 3
        )

    def protected(n, mask_only=False, frame=None):
        (width, height), p = frame if frame is not None else shot(n)
        if p[:3] == mask:
            assert p == mask * (width * height), "non-mask pixels on blanked head"
            return "mask"
        assert not mask_only and desktop * 8 not in p, ("unprotected output", n)
        ref = wallpaper(width, height)
        # Check the sides, top and bottom against the wallpaper, including the
        # former taskbar and desktop icons. The central clock and form are the
        # only admitted UI. A background app's solid colour is forbidden anywhere.
        arrow_samples = 0
        for y in range(9, height, 23):
            for x in range(7, width, 23):
                if abs(x - width / 2) < 195 and height * 0.18 < y < height * 0.92:
                    continue
                off = 3 * (y * width + x)
                if (
                    p[off : off + 3] in (bytes(3), bytes([255]) * 3)
                    and max(
                        abs(a - b) for a, b in zip(p[off : off + 3], ref[off : off + 3])
                    )
                    >= 12
                ):
                    arrow_samples += 1
                    continue
                assert (
                    max(
                        abs(a - b) for a, b in zip(p[off : off + 3], ref[off : off + 3])
                    )
                    < 12
                ), ("unexpected pixels", n, x, y, p[off : off + 3], ref[off : off + 3])
        # Our built-in 14×21 black-and-white arrow fits between adjacent probes.
        assert arrow_samples <= 1, ("unexpected cursor pixels", n, arrow_samples)
        return "prompt"

    def password(keys):
        for key in keys:
            g.key(key)
            time.sleep(0.12)

    # Serialize QMP commands while a sampler checks both heads across the deaths.
    qmp = g.qmp
    mutex = threading.Lock()

    def serial_qmp(*args, **kwargs):
        with mutex:
            return qmp(*args, **kwargs)

    g.qmp = serial_qmp
    stop = threading.Event()
    samples = []
    errors = []

    def sample():
        try:
            while not stop.is_set():
                samples.append(tuple(protected(n) for n in (0, 1)))
                stop.wait(0.08)
        except Exception as error:  # noqa: BLE001 - re-raise sampler failures on the test thread
            errors.append(error)

    sampler = None
    try:
        plug(2, True)
        s.joe(
            "busctl --user call org.joelinux.MundaneSessionTest /org/joelinux/MundaneSessionTest org.freedesktop.Application Activate 'a{sv}' 0"
        )
        wait("test client", lambda: "frame mundane session probe" in s.tree())
        g.key("a")
        wait(
            "background input and pixels",
            lambda: (
                desktop in shot(0)[1] and "key a" in s.read("mundane-login-test.log")
            ),
        )
        wait("recording Orca", lambda: "Screen reader on." in speech())
        # Delay the first exec and the replacement independently of the compositor.
        g.shell(f"mv {exe} {held}")
        start = time.monotonic()
        g.key("meta_l", "l")
        wait("all heads protected", lambda: g.shell(hint) == "yes")
        assert not pid()
        for n in (0, 1):
            protected(n, mask_only=True)
        sampler = threading.Thread(target=sample)
        sampler.start()
        original = s.read("mundane-login-test.log")
        g.key("b")
        g.click(0.50, 0.50)
        offset = len(speech())
        g.shell(f"mv {held} {exe}")
        wait(
            "first admission",
            lambda: protected(0) == "prompt" and protected(1) == "prompt",
        )
        print(
            f"First protected prompt {time.monotonic() - start:.3f} s including delayed exec",
            flush=True,
        )
        wait(
            "locker focus and Orca speech",
            lambda: any("Password" in t for t in speech()[offset:]),
        )
        s.shot("locked")
        assert s.tree().count("password text Password") == 1 and any(
            "Keyboard ·" in row for row in s.tree()
        )
        g.move(0.96, 0.35)
        wait(
            "prompt follows pointer to second head",
            lambda: (
                bytes.fromhex("17201e") * 120 in shot(1)[1]
                and bytes.fromhex("17201e") * 120 not in shot(0)[1]
            ),
        )
        g.move(0.2, 0.35)
        wait("prompt returns", lambda: bytes.fromhex("17201e") * 120 in shot(0)[1])
        # Observe actual opacity on scanout, with and without the motion preference.
        for reduced in (False, True):
            s.pref("interface enable-animations", "false" if reduced else "true")
            g.key("shift")
            time.sleep(0.5)
            x, y, w, h = map(int, s.widget("bounds", "Unlock").split())
            pixel = 3 * ((y + h // 2) * 1920 + x + 12)

            def ink(pixel=pixel):
                return shot(0)[1][pixel : pixel + 3]

            full = ink()
            time.sleep(28)
            colors = {full}
            until = time.monotonic() + 4
            while time.monotonic() < until:
                colors.add(ink())
            empty = ink()
            assert full != empty, (x, y, w, h, full)
            intermediate = [
                c
                for c in colors
                if min(sum(abs(a - b) for a, b in zip(c, end)) for end in (full, empty))
                > 12
            ]
            assert bool(intermediate) != reduced, (reduced, colors)
            assert "password text Password" not in s.tree()
            s.shot("clock")
            if reduced:
                g.move(0.25, 0.41)
            else:
                g.key("x")
            wait(
                "wake restores the protected field",
                lambda: "password text Password" in s.tree(),
            )
        s.pref("interface enable-animations", "true")
        first = pid()
        print(
            "Locker memory:",
            g.shell(f"grep -E '^(Pss|Rss):' /proc/{int(first)}/smaps_rollup"),
            flush=True,
        )
        offset = len(speech())
        password(("x", "9", "z"))
        assert (
            s.joe(f"python3 -W ignore {FIX}/shell_widgets.py protected Password")
            == "protected"
        )
        g.key("ret")
        wait(
            "inline PAM denial", lambda: any("Unlock failed" in row for row in s.tree())
        )
        wait(
            "Orca reads denial",
            lambda: any("Unlock failed" in row for row in speech()[offset:]),
        )
        assert not set(speech()[offset:]) & {"x", "9", "z", "x9z"}
        assert g.shell(hint) == "yes" and s.read("mundane-login-test.log") == original
        s.shot("denied")
        g.shell(f"mv {exe} {held} && kill -KILL {int(first)}")
        wait(
            "both outputs returned to the mask",
            lambda: protected(0) == "mask" and protected(1) == "mask",
        )
        # A new scanout must stay covered even without any locker executable.
        inactive = shot(2)[1]
        plug(3, True)
        wait("third head active", lambda: shot(2)[1] != inactive)
        protected(2, mask_only=True)
        shell = int(g.shell("pgrep -x -u joe mundane-shell"))
        g.shell(f"kill -KILL {shell}")
        wait(
            "shell replaced while locked",
            lambda: (
                (p := g.shell("pgrep -x -u joe mundane-shell || :")).isdigit()
                and int(p) != shell
            ),
        )
        assert g.shell(hint) == "yes"
        g.key("meta_l", "spc")
        g.click(0.5, 0.5)
        assert s.read("mundane-login-test.log") == original
        for n in range(3):
            protected(n, mask_only=True)
        g.shell(f"mv {held} {exe}")
        wait(
            "fresh locker on all three heads",
            lambda: all(protected(n) == "prompt" for n in range(3)),
        )
        assert pid() != first
        plug(3, False)
        plug(3, True)
        wait(
            "reconnected output admitted",
            lambda: looks_locked(shot(2)) and protected(2) == "prompt",
        )
        stop.set()
        sampler.join()
        assert not errors and len(samples) >= 2, (len(samples), errors)
        vt = int(g.shell(f"loginctl show-session {sid} -p VTNr --value"))
        away = 6 if vt != 6 else 5
        before_vt = pid()
        g.key("ctrl", "alt", f"f{away}")
        wait(
            "VT released",
            lambda: g.shell("cat /sys/class/tty/tty0/active") == f"tty{away}",
        )
        g.key("z")
        time.sleep(0.15)
        away_frames = [[shot(n)[1]] for n in range(3)]
        time.sleep(0.55)
        for n in range(3):
            away_frames[n].append(shot(n)[1])
        g.shell(f"chvt {vt}")
        wait(
            "VT reacquired",
            lambda: g.shell(f"loginctl show-session {sid} -p Active --value") == "yes",
        )

        def reacquired(n):
            frame = shot(n)
            pixels = frame[1]
            if looks_locked(frame):
                return protected(n, frame=frame)
            if (
                pixels in away_frames[n]
                or pixels == inactive
                or pixels == bytes(len(pixels))
            ):
                return False
            raise AssertionError(("unexpected VT scanout", n, pixels[:3].hex()))

        for n in range(3):
            wait("protected VT return on head " + str(n), lambda n=n: reacquired(n))
        wait(
            "new PAM conversation after VT",
            lambda: pid().isdigit() and pid() != before_vt,
        )
        before_sleep = pid()
        g.shell("systemctl suspend --no-block")
        wait("guest S3", lambda: g.qmp("query-status")["status"] == "suspended")
        g.qmp("system_wakeup")
        g.sync_console()
        wait("resume", lambda: g.shell(hint) == "yes" and looks_locked(shot(0)))
        for n in range(3):
            wait("protected resume on head " + str(n), lambda n=n: reacquired(n))
        wait(
            "fresh post-resume prompt",
            lambda: protected(0) == "prompt" and "password text Password" in s.tree(),
        )
        assert pid().isdigit() and pid() != before_sleep
        assert s.read("mundane-login-test.log") == original
        password(("j", "o", "e", "ret"))
        wait("real PAM success", lambda: g.shell(hint) == "no" and not pid())
        g.key("d")
        wait(
            "original app regains keyboard",
            lambda: "key d" in s.read("mundane-login-test.log")[len(original) :],
        )
        # Normal two-output startup, without delaying exec or probing the serial console.
        plug(3, False)
        wait(
            "third output removed",
            lambda: (
                len(json.loads(s.joe("/usr/local/bin/mundane-de output list"))) == 2
            ),
        )
        # A valid password cannot override a failing PAM account policy.
        g.shell(
            f"cp -p {pam_service} {original_service} && echo 'account required pam_deny.so' >> {pam_service}"
        )
        start = time.monotonic()
        g.key("meta_l", "l")
        timings = {}
        while len(timings) < 2 and time.monotonic() - start < 8:
            frame = shot(0)
            if looks_locked(frame):
                kind = protected(0, frame=frame)
                timings.setdefault("covered", time.monotonic() - start)
                if kind == "prompt":
                    timings["prompt"] = time.monotonic() - start
            time.sleep(0.008)
        assert len(timings) == 2, timings
        wait("prompt on second output", lambda: protected(1) == "prompt")
        print(
            f"Normal lock, two outputs: first protected frame {timings['covered']:.3f} s, prompt {timings['prompt']:.3f} s.",
            flush=True,
        )
        print(
            "Additional locker memory:",
            g.shell(f"grep -E '^(Pss|Rss):' /proc/{int(pid())}/smaps_rollup"),
            flush=True,
        )
        g.move(0.1, 0.2)
        for appearance, scheme, contrast, backdrop in (
            ("light", "prefer-light", "false", "f8faf8"),
            ("contrast", "prefer-dark", "true", "000000"),
        ):
            s.pref("interface color-scheme", scheme)
            s.pref("a11y.interface high-contrast", contrast)
            wait(
                "active prompt appearance",
                lambda backdrop=backdrop: bytes.fromhex(backdrop) * 80 in shot(0)[1],
            )
            protected(0)
            protected(1)
            s.shot("locked-" + appearance)
        password(("j", "o", "e", "ret"))
        wait(
            "valid password denied by account policy",
            lambda: any("Unlock failed" in row for row in s.tree()),
        )
        assert g.shell(hint) == "yes"
        refused = pid()
        g.shell(f"mv {original_service} {pam_service} && kill -TERM {int(refused)}")
        wait(
            "policy restored with fresh conversation",
            lambda: (
                pid().isdigit()
                and pid() != refused
                and "password text Password" in s.tree()
            ),
        )
        password(("j", "o", "e", "ret"))
        wait("second PAM unlock", lambda: g.shell(hint) == "no" and not pid())
        assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
        print(
            f"{len(samples)} paired scanouts through locker and shell loss. Three outputs, hotplug, PAM denial and success, speech, VT, S3 and input isolation passed.",
            flush=True,
        )
    finally:
        stop.set()
        if sampler:
            sampler.join()
        g.shell(f"if test -e {held}; then mv {held} {exe}; fi")
        g.shell(
            f"if test -e {original_service}; then mv {original_service} {pam_service}; fi"
        )
        for n in (3, 2):
            plug(n, False)
        for key, value in old.items():
            s.joe(f"gsettings set {key} {value}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("screenshots", type=Path)
    args = parser.parse_args()
    qualify(args.run, args.screenshots)
