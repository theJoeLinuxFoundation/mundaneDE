"""Check Orca's speech on the active seat of a headless joe test overlay."""

import argparse
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import time
from guest import Guest

HERE = Path(__file__).resolve().parent


def wait(description, fn, seconds=15):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = fn()
        if result:
            return result
        time.sleep(0.15)
    raise AssertionError(description)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--echo", type=Path, required=True, help="GPUI echo example")
    parser.add_argument(
        "--suite-target",
        type=Path,
        required=True,
        help="pinned suite build containing mundane-terminal and examples/motion",
    )
    opts = parser.parse_args()
    g = Guest(opts.run)
    assert g.qmp("query-display-options")["type"] == "none"
    blocks = {v["node-name"]: v["ro"] for v in g.qmp("query-named-block-nodes")}
    assert blocks["base"] and blocks["basefile"] and not blocks["layer"]
    assert g.shell("pgrep -x -u joe mundane-shell")
    assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
    share = g.run / "share"
    for path in [*(HERE / "accessibility").glob("*.py"), HERE / "shell_widgets.py"]:
        shutil.copy(path, share / path.name)
    for source, name in (
        (opts.echo, "m7-echo"),
        (opts.suite_target / "mundane-terminal", "mundane-terminal"),
        (opts.suite_target / "examples/motion", "suite-motion"),
    ):
        subprocess.run(["objcopy", "--strip-debug", source, share / name], check=True)
    g.shell(
        "mkdir -p /tmp/m7; mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt; cp /mnt/*.py /mnt/m7-echo /mnt/suite-motion /tmp/m7/; install -m755 /mnt/mundane-terminal /usr/local/bin/; umount /mnt; chmod 755 /tmp/m7/m7-echo /tmp/m7/suite-motion; install -d -o joe -g users /home/joe/.local/share/orca"
    )
    g.shell(
        "cp /tmp/m7/recording_speech.py /tmp/m7/orca-customizations.py /home/joe/.local/share/orca/"
    )
    rt = "/run/user/1000"

    def joe(cmd):
        return g.shell(
            "su joe -s /bin/sh -c "
            + shlex.quote(
                f"export XDG_RUNTIME_DIR={rt} DBUS_SESSION_BUS_ADDRESS=unix:path={rt}/bus WAYLAND_DISPLAY=wayland-1 XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=mundane; "
                + cmd
            )
        )

    def launch(cmd, name):
        return int(joe(f"{cmd} </dev/null >/tmp/m7/{name}.log 2>&1 & echo $!"))

    def read(name):
        return g.shell(f"cat {rt}/{name} 2>/dev/null || :")

    def speech():
        return [json.loads(l)[1] for l in read("mundane-speech.jsonl").splitlines()]

    def tree():
        return joe("python3 -W ignore /tmp/m7/shell_widgets.py list")

    g.shell("chmod 1777 /tmp/m7")
    pref = "gsettings set org.gnome.desktop.a11y.applications screen-reader-enabled "
    joe(pref + "false")
    wait(
        "Orca exited",
        lambda: not g.shell("pgrep -u joe -f '^/usr/bin/python3 /usr/bin/orca' || :"),
    )
    g.key("meta_l", "alt", "s")
    wait(
        "session launched Orca",
        lambda: (
            joe(
                "gsettings get org.gnome.desktop.a11y.applications screen-reader-enabled"
            )
            == "true"
            and g.shell("pgrep -u joe -f '^/usr/bin/python3 /usr/bin/orca' || :")
        ),
    )
    denied = joe(
        "gdbus call --session --dest org.freedesktop.a11y.Manager --object-path /org/freedesktop/a11y/Manager --method org.freedesktop.a11y.KeyboardMonitor.WatchKeyboard 2>&1 || :"
    )
    assert "AccessDenied" in denied, denied
    joe(f"rm -f {rt}/observer-ready {rt}/unexpected-monitor-event")
    observer = launch("python3 /tmp/m7/observe.py", "observe")
    wait(
        "observer registered",
        lambda: g.shell(f"test -e {rt}/observer-ready && echo yes || :"),
    )
    try:
        for cmd in ("python3 /tmp/m7/entry.py", "/tmp/m7/m7-echo"):
            for physical in (True, False):
                joe(f"rm -f {rt}/echo-value {rt}/echo-inserts {rt}/append-character")
                # Orca retains the last printable key across focused applications.
                # Give each independent trial fresh input history.
                before = len(speech())
                joe(pref + "false")
                wait(
                    "reader stopped",
                    lambda: (
                        not g.shell(
                            "pgrep -u joe -f '^/usr/bin/python3 /usr/bin/orca' || :"
                        )
                    ),
                )
                joe(pref + "true")
                wait(
                    "reader initialized",
                    lambda: "Screen reader on." in speech()[before:],
                )
                pid = launch(cmd, "entry")
                try:
                    wait("field registered: " + cmd, lambda: "City" in tree())
                    time.sleep(1.0)
                    offset = len(speech())
                    if physical:
                        g.key("z")
                    else:
                        joe(f"touch {rt}/append-character")
                    wait(
                        "Tokyoz and insert",
                        lambda: (
                            read("echo-value") == "Tokyoz"
                            and '[5, 1, "z"]' in read("echo-inserts")
                        ),
                    )
                    time.sleep(0.8)
                    spoken = speech()[offset:]
                    assert spoken.count("z") == int(physical), (cmd, physical, spoken)
                    print(
                        cmd,
                        "typed" if physical else "programmatic",
                        "Tokyoz",
                        spoken,
                        flush=True,
                    )
                finally:
                    g.shell(f"kill {pid} 2>/dev/null || :")
                wait("field closed", lambda: "City" not in tree())
        assert not g.shell(f"test -e {rt}/unexpected-monitor-event && echo leaked || :")
    finally:
        g.shell(f"kill {observer} 2>/dev/null || :")
    from accessibility.desktop_checks import check

    check(g, joe, launch, tree, speech, read, wait)
    from accessibility.gpui_agent import qualify

    qualify(g, joe, launch, tree, speech, wait)
    old = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
    g.shell("systemctl restart gdm")

    def logged_in():
        sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
        return (
            sid != old
            and sid.isdigit()
            and g.shell(f"loginctl show-session {sid} -p Name --value") == "joe"
        )

    wait("fresh GDM login", logged_in, 60)
    wait(
        "reader restored",
        lambda: g.shell("pgrep -u joe -f '^/usr/bin/python3 /usr/bin/orca' || :"),
    )
    assert joe("gsettings get org.gnome.desktop.interface enable-animations") == "false"
    assert (
        joe("gsettings get org.gnome.desktop.interface color-scheme") == "'prefer-dark'"
    )
    print("Reader, reduced motion and dark appearance persisted across GDM login.")


if __name__ == "__main__":
    main()
