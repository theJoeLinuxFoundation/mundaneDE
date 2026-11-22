"""Drive notifications, deployment notices and media keys on a dedicated headless joe guest."""

import argparse
import hashlib
import json
import shlex
import shutil
import time
from pathlib import Path

from qualify_gpui_settings import Session
from qualify_menus import wait


def snapshot(revision, prepared=None, outcome=None):
    last = (
        {
            "event": "deployment-5",
            "deployment": 5,
            "outcome": outcome,
            "message": "Boot check",
        }
        if outcome
        else None
    )
    return dict(
        schema="dave.status/1",
        revision=str(revision),
        managed=True,
        complete=True,
        running=4,
        next_boot=4,
        deployments=[],
        prepared_upgrade=prepared,
        last_upgrade=last,
    )


FIX = "/tmp/mundane-feedback-test"


def notifications(s):
    g = s.g
    shutil.copyfile(
        Path(__file__).with_name("notification_client.py"),
        g.run / "share" / "notification_client.py",
    )
    g.shell(
        f"install -d {FIX}; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && cp /mnt/notification_client.py {FIX}/ && umount /mnt"
    )
    cmd = f"python3 -u {FIX}/notification_client.py"
    pref = "org.gnome.desktop.notifications show-banners"
    previous = s.joe("gsettings get " + pref)
    s.joe("gsettings set " + pref + " true")
    pid = s.launch(cmd + " copy", "step3-note")

    def log():
        return g.shell("cat /tmp/mundane-step3-note.log")

    try:
        wait("replacement banner", lambda: "button Open Pictures" in s.tree())
        assert "article Copying files" not in s.tree()
        scheme = "org.gnome.desktop.interface color-scheme"
        contrast = "org.gnome.desktop.a11y.interface high-contrast"
        original_scheme = s.joe("gsettings get " + scheme)
        original_contrast = s.joe("gsettings get " + contrast)
        try:
            for name, theme, high in [
                ("notification", "prefer-light", "false"),
                ("dark-notification", "prefer-dark", "false"),
                ("contrast-notification", "prefer-dark", "true"),
            ]:
                s.joe(
                    f"gsettings set {scheme} {theme} && gsettings set {contrast} {high}"
                )
                s.shot(name)
        finally:
            s.joe(
                f"gsettings set {scheme} {original_scheme} && gsettings set {contrast} {original_contrast}"
            )

        note_id = int(
            next(
                line.split()[1] for line in log().splitlines() if line.startswith("ID ")
            )
        )
        s.joe("gsettings set " + pref + " false")
        wait("existing banner suppressed", lambda: "frame Notification" not in s.tree())
        s.joe(cmd + " foreign " + str(note_id))
        assert "frame Notification" not in s.tree()
        s.key("meta_l", "s")
        wait("two distinct owners", lambda: "button Notifications (2)" in s.tree())
        s.click("Notifications (2)")
        wait(
            "history displayed",
            lambda: (
                "article Copy complete" in s.tree() and "article Calendar" in s.tree()
            ),
        )
        s.shot("notification-history")
        s.key("down")
        s.key("end")
        s.click("Open Pictures")
        wait(
            "action to original sender",
            lambda: (
                f"ActionInvoked {note_id} open" in log()
                and f"NotificationClosed {note_id} 2" in log()
            ),
        )
        s.click("Dismiss")
        wait(
            "dismissed remaining card",
            lambda: "status bar No notifications" in s.tree(),
        )
        s.key("esc")
        assert "Owner closed its notification" in s.joe(cmd + " close")
        s.joe(cmd + " fill")
        s.key("meta_l", "s")
        wait("bounded history", lambda: "button Notifications (64)" in s.tree())
        s.click("Notifications (64)")
        wait(
            "last 64 items",
            lambda: (
                "article Transfer 4" in s.tree() and "article Transfer 67" in s.tree()
            ),
        )
        assert "article Transfer 3" not in s.tree()
        s.key("esc")
        s.joe("gsettings set " + pref + " true")
        s.joe(cmd + " short")
        wait("timed banner", lambda: "article Transfer complete" in s.tree())
        wait("timed banner expired", lambda: "frame Notification" not in s.tree())
        s.key("meta_l", "s")
        s.click("Notifications (64)")
        wait("expired banner retained", lambda: "article Transfer complete" in s.tree())
        s.key("esc")
    finally:
        s.joe("gsettings set " + pref + " " + previous)
        g.shell(f"kill {pid} 2>/dev/null || :")
    print(
        "Owner-scoped replacement and close, action delivery, timed banners, Do Not Disturb and 64-card history passed.",
        flush=True,
    )


def levels(s):
    module = s.joe("pactl load-module module-null-sink sink_name=step3_audio")
    try:
        s.joe(
            "wpctl set-mute @DEFAULT_AUDIO_SINK@ 0 && wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.45"
        )
        s.joe(
            "busctl --user call org.joelinux.MundaneSessionTest /org/joelinux/MundaneSessionTest org.freedesktop.Application Activate 'a{sv}' 0"
        )
        log = lambda: s.g.shell("cat /run/user/1000/mundane-login-test.log || :")
        wait("typing probe focused", lambda: "frame mundane session probe" in s.tree())
        s.key("a")
        before = log().splitlines().count("key b")
        s.key("volumeup")
        wait("volume meter", lambda: "frame Volume" in s.tree())
        s.key("b")
        assert "0.50" in s.joe("wpctl get-volume @DEFAULT_AUDIO_SINK@")
        assert log().splitlines().count("key b") == before + 1
        s.shot("volume-osd")
        s.key("audiomute")
        wait("mute feedback", lambda: any("Muted" in item for item in s.tree()))
        wait("OSD expired", lambda: "frame Volume" not in s.tree())
        s.key("meta_l", "q")
    finally:
        s.joe("pactl unload-module " + module)
    print(
        "Volume and mute reached PipeWire; typing continued through the visible OSD.",
        flush=True,
    )


def upgrades(s):
    # Interpose dave only for this guest shell. Keep the installed dave and its store intact.
    g = s.g
    binary = "/usr/local/bin/mundane-shell"

    def publish(value):
        g.shell(
            f"printf '%s' {shlex.quote(json.dumps(value))} >{FIX}/next && mv {FIX}/next {FIX}/status"
        )

    g.shell(f"install -d {FIX}/bin")
    stamp = hashlib.sha256(str(time.time_ns()).encode()).hexdigest()
    stage = {
        "id": "sha256:" + stamp,
        "install": 2,
        "remove": 0,
        "summary": "2 packages prepared",
    }
    publish(snapshot(1, stage))
    script = (
        f'#!/bin/sh\ntest "$*" = "status --json" || exit 1\nexec cat {FIX}/status\n'
    )
    wrapper = f'#!/bin/sh\nexport PATH={FIX}/bin:$PATH\nexec {binary}.real "$@"\n'
    g.shell(
        f"set -e; test ! -e {binary}.real; printf %s {shlex.quote(script)} >{FIX}/bin/dave; chmod 755 {FIX}/bin/dave; mv {binary} {binary}.real; printf %s {shlex.quote(wrapper)} >{binary}; chmod 755 {binary}"
    )

    # Linux comm for the wrapped executable follows its basename.
    def wrapped_restart():
        pid = int(g.shell("pgrep -u joe -f '^/usr/local/bin/mundane-shell(\\.real)?$'"))
        g.shell(f"kill {pid}")
        wait(
            "replacement shell",
            lambda: g.shell(
                f"ps -u joe -o pid=,args= | grep -E ' /usr/local/bin/mundane-shell.real$' | grep -v '^ *{pid} ' || :"
            ),
        )

    try:
        wrapped_restart()
        wait(
            "prepared banner",
            lambda: "article Upgrade ready, restart to apply" in s.tree(),
            25,
        )
        wrapped_restart()
        wait("new taskbar", lambda: "button Start" in s.tree())
        s.key("meta_l", "s")
        wait(
            "deduplicated prepared status",
            lambda: (
                "label 2 packages prepared" in s.tree()
                and "button Notifications (0)" in s.tree()
            ),
        )
        s.key("esc")
        publish(snapshot(2, outcome="not_running"))
        time.sleep(16)
        assert "frame Notification" not in s.tree()
        s.key("meta_l", "s")
        wait(
            "manual rollback stays quiet",
            lambda: (
                "button Notifications (0)" in s.tree()
                and "label 2 packages prepared" not in s.tree()
            ),
        )
        s.key("esc")
        fallback = snapshot(3, outcome="fallback")
        fallback["last_upgrade"]["event"] = stamp
        publish(fallback)
        wait(
            "automatic fallback",
            lambda: (
                any("Returned to deployment 4." in row for row in s.tree())
                and "article Upgrade to deployment 5 failed" in s.tree()
            ),
            25,
        )
        s.shot("upgrade-fallback")
        wrapped_restart()
        wait("new taskbar", lambda: "button Start" in s.tree())
        s.key("meta_l", "s")
        wait(
            "persistent fallback details",
            lambda: (
                any("Returned to deployment 4." in row for row in s.tree())
                and "button Notifications (0)" in s.tree()
            ),
        )
        s.key("esc")
    finally:
        g.shell(f"mv -f {binary}.real {binary}; systemctl restart gdm")
        s.ready()
    print(
        "Prepared and fallback notices survived restart without repeating; manual rollback stayed quiet.",
        flush=True,
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("screenshots", type=Path)
    p.add_argument("--only", choices=["notifications", "levels", "upgrades"])
    args = p.parse_args()
    session = Session(args.run, args.screenshots)
    for check in [notifications, levels, upgrades]:
        if not args.only or args.only == check.__name__:
            check(session)
