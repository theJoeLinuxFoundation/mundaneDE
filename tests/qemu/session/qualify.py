#!/usr/bin/env python3
"""Exercise GDM, seat ownership, sleep and logout on a prepared joe --test overlay."""

import argparse
from pathlib import Path
import re
import shlex
import subprocess
import time

from guest import Guest

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
args = parser.parse_args()
g = Guest(args.run)
runtime = "/run/user/1000"
events = runtime + "/mundane-login-test.log"


def wait(label, probe, seconds=30):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        value = probe()
        if value:
            return value
        time.sleep(0.15)
    raise RuntimeError("timeout: " + label)


def as_joe(command):
    # This serial-side observer addresses the user bus. Only the activation
    # environment published by the session supplies a display to its clients.
    return g.shell(
        "su joe -s /bin/sh -c "
        + shlex.quote(
            f"unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET XDG_CURRENT_DESKTOP XDG_SESSION_ID; "
            f"export XDG_RUNTIME_DIR={runtime} DBUS_SESSION_BUS_ADDRESS=unix:path={runtime}/bus; {command}"
        )
    )


def probe_log():
    return g.shell("cat " + events + " 2>/dev/null || :")


def color(label, hexvalue):
    return g.screen(label).count(bytes.fromhex(hexvalue)) >= 45000


def key_frame(label, key, expected):
    g.key(key)
    wait(label + " keyboard", lambda: "key " + key in probe_log().splitlines())
    wait(label + " scanout", lambda: color(label, expected))


def click_probe():
    before = probe_log().splitlines().count("click")
    # A tablet can retain its previous absolute position across GDM logins.
    g.move(0.35, 0.35)
    g.click(0.5, 0.5)
    wait("pointer delivered", lambda: probe_log().splitlines().count("click") > before)


def login(index):
    def session_id():
        s = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
        if not s or not s.isdigit():
            return None
        active = g.shell(
            f"loginctl show-session {s} -p Name -p Type -p Active -p Class -p Seat"
        )
        return (
            s
            if {"Name=joe", "Type=wayland", "Active=yes", "Class=user", "Seat=seat0"}
            <= set(active.splitlines())
            else None
        )

    sid = wait("GDM login", session_id, 65)
    wait(
        "compositor and agent",
        lambda: (
            g.shell(
                "pgrep -u joe -f '^/usr/local/bin/mundane-de --session-notify ' && pgrep -u joe -x mundane-shell || :"
            ).count("\n")
            == 1
        ),
    )
    pid = int(g.shell("pgrep -u joe -f '^/usr/local/bin/mundane-de --session-notify '"))
    agent = int(g.shell("pgrep -u joe -x mundane-shell"))
    leader = int(g.shell("pgrep -u joe -x mundane-session"))
    assert g.shell(f"readlink /proc/{leader}/exe") == "/usr/local/bin/mundane-session"
    assert g.shell(f"readlink /proc/{agent}/exe") == "/usr/local/bin/mundane-shell"
    for p in (leader, pid, agent):
        assert f"session-{sid}.scope" in g.shell(f"cat /proc/{p}/cgroup")
    assert as_joe(
        "systemctl --user is-active mundane-session.target graphical-session.target"
    ).splitlines() == ["active", "active"]
    env = dict(
        line.split("=", 1)
        for line in as_joe("systemctl --user show-environment").splitlines()
        if "=" in line
    )
    assert (
        env["XDG_CURRENT_DESKTOP"],
        env["XDG_SESSION_DESKTOP"],
        env["XDG_SESSION_TYPE"],
        env["XDG_SESSION_ID"],
    ) == ("mundane", "mundane", "wayland", sid)
    wait(
        "public portal and GTK backend on this login",
        lambda: (
            as_joe(
                "systemctl --user is-active xdg-desktop-portal.service xdg-desktop-portal-gtk.service || :"
            ).splitlines()
            == ["active", "active"]
        ),
        30,
    )
    name = env["WAYLAND_DISPLAY"]
    assert re.fullmatch("wayland-[0-9]+", name)
    socket = f"{runtime}/{name}"
    inode = int(g.shell(f"test -S {socket} && stat -c %i {socket}"))
    fds = g.shell(f"ls -l /proc/{pid}/fd")
    assert "/dev/dri/card" in fds and "/dev/input/event" in fds
    portal = as_joe(
        "busctl --user get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.FileChooser version"
    )
    assert portal.startswith("u ") and int(portal[2:]) > 0
    g.shell(f"rm -f {events}")
    as_joe(
        "busctl --user call org.joelinux.MundaneSessionTest /org/joelinux/MundaneSessionTest org.freedesktop.Application Activate 'a{sv}' 0"
    )
    ready = wait(
        "D-Bus activation",
        lambda: next(
            (l for l in probe_log().splitlines() if l.startswith("ready ")), None
        ),
    )
    _, app, uid, display, session = ready.split()
    assert (uid, display, session) == ("1000", name, sid)
    wait("activated GTK on scanout", lambda: color(f"login-{index}", "20df61"))
    key_frame(f"login-{index}-key", "a", "2088df")
    click_probe()
    print(
        f"Login {index}: GDM session {sid}, compositor {pid}, polkit {agent}, GTK {app}, socket {name} inode {inode}",
        flush=True,
    )
    return sid, pid, agent, int(app), socket, inode


def power_cycle(data):
    sid, pid, *_ = data
    active = int(g.shell(f"loginctl show-session {sid} -p VTNr --value"))
    other = 6 if active != 6 else 5
    g.key("ctrl", "alt", f"f{other}")
    wait(
        "released VT",
        lambda: g.shell("cat /sys/class/tty/tty0/active") == f"tty{other}",
    )
    assert g.shell(f"loginctl show-session {sid} -p Active --value") == "no"
    wait(
        "input fds released",
        lambda: "/dev/input/event" not in g.shell(f"ls -l /proc/{pid}/fd"),
    )
    text = probe_log()
    g.key("z")
    g.screen("away-vt")
    assert probe_log() == text
    g.shell(f"chvt {active}")
    wait(
        "seat reactivated",
        lambda: (
            g.shell(f"loginctl show-session {sid} -p Active --value") == "yes"
            and "/dev/input/event" in g.shell(f"ls -l /proc/{pid}/fd")
        ),
    )
    key_frame("vt-return", "b", "df8520")
    click_probe()
    print(
        f"VT {active} -> {other} -> {active}: devices released, keyboard, pointer and changed scanout verified",
        flush=True,
    )
    assert g.qmp("query-current-machine")["wakeup-suspend-support"]
    g.shell("systemctl suspend --no-block")
    wait("ACPI suspend", lambda: g.qmp("query-status")["status"] == "suspended")
    g.qmp("system_wakeup")
    wait("ACPI wake", lambda: g.qmp("query-status")["running"])
    g.sync_console()
    wait(
        "guest kernel resume",
        lambda: (
            "PM: suspend exit"
            in g.shell("journalctl -k -b -o cat --no-pager | grep 'PM: suspend' || :")
        ),
    )
    wait(
        "resume remains locked",
        lambda: g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "yes",
    )
    as_joe(
        "busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true"
    )
    g.move(0.26, 0.31)
    wait(
        "PAM conversation ready",
        lambda: (
            "password text Password"
            in as_joe("python3 -W ignore /usr/local/libexec/mundane-test-widgets list")
        ),
    )
    for key in ("j", "o", "e", "ret"):
        g.key(key)
        time.sleep(0.12)
    wait(
        "PAM unlock after resume",
        lambda: g.shell(f"loginctl show-session {sid} -p LockedHint --value") == "no",
    )
    key_frame("s3-return", "c", "c12387")
    click_probe()
    print(
        "ACPI S3 and QMP system_wakeup: changed scanout, keyboard and pointer verified",
        flush=True,
    )


def settings_auth():

    old = g.shell("timedatectl show -p Timezone --value")
    assert old == "UTC", old
    g.key("meta_l", "comma")
    wait(
        "Settings mapped",
        lambda: (
            "/usr/local/bin/mundane-settings"
            in g.shell(
                "p=$(pgrep -u joe -x mundane-settings || pgrep -u joe -f '^mundane-settings$'); readlink /proc/$p/exe || :"
            )
        ),
    )

    def widget(verb, name=""):
        return as_joe(
            "python3 -W ignore /usr/local/libexec/mundane-test-widgets "
            + shlex.join([verb, name])
        )

    as_joe(
        "gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled '<true>'"
    )
    wait("Date & Time in Settings", lambda: "button Date & Time" in widget("list"))
    widget("click", "Date & Time")

    def choose():
        wait("timezone control", lambda: "button Timezone" in widget("list"))
        widget("click", "Timezone")
        wait("Cairo in timezone list", lambda: "button Africa/Cairo" in widget("list"))
        widget("click", "Africa/Cairo")
        wait(
            "shell password prompt", lambda: "password text Password" in widget("list")
        )

    choose()
    widget("click", "Cancel")
    wait("cancel dismissed request", lambda: not g.shell("pgrep -x pkexec || :"))
    assert g.shell("timedatectl show -p Timezone --value") == old
    choose()
    for key in ("j", "o", "e", "ret"):
        g.key(key)
        time.sleep(0.14)
    wait(
        "authorized timezone change",
        lambda: g.shell("timedatectl show -p Timezone --value") == "Africa/Cairo",
    )
    assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
    g.screen("settings-authorized")
    print(
        "Settings: graphical polkit cancellation kept UTC; PAM authentication changed the timezone to Africa/Cairo under enforcement",
        flush=True,
    )
    g.key("meta_l", "q")


def logout(data):
    sid, pid, agent, app, sock, _ = data
    g.key("meta_l", "shift", "q")
    wait(
        "old session processes and sockets removed",
        lambda: (
            g.shell(
                f"for p in {pid} {agent} {app}; do test ! -d /proc/$p || exit 0; done; "
                f"for p in {sock} {sock}.lock {sock}.outputs {sock}.x11; do test ! -e $p || exit 0; done; echo gone"
            )
            == "gone"
        ),
    )
    if g.shell(f"test -S {runtime}/bus && echo up || :") == "up":
        manager = as_joe("systemctl --user show-environment")
        assert not re.search(
            r"^(WAYLAND_DISPLAY|XDG_CURRENT_DESKTOP|XDG_SESSION_DESKTOP|XDG_SESSION_ID|DISPLAY|XAUTHORITY)=",
            manager,
            re.M,
        )
        assert (
            as_joe(
                "systemctl --user is-active mundane-session.target mundane-ibus.service "
                "graphical-session.target xdg-desktop-portal.service || :"
            ).splitlines()
            == ["inactive"] * 4
        )
    greeter = wait(
        "GDM greeter session",
        lambda: g.shell(
            'loginctl list-sessions --no-legend | awk \'$5 == "greeter" || $6 == "greeter" {print $1}\''
        ),
    )
    g.screen(f"logout-{sid}-greeter")
    print(
        f"Logout {sid}: old processes, sockets and activation environment gone; GDM greeter {greeter}",
        flush=True,
    )


assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
first = login(1)
power_cycle(first)
settings_auth()
logins = [first]
for index in (2, 3):
    logout(logins[-1])
    # GDM does not automatically log the user back in after an explicit
    # logout. Restart the test image's autologin display manager at the greeter.
    g.shell("systemctl restart gdm")
    logins.append(login(index))
assert len({r[0] for r in logins}) == 3 and len({r[1] for r in logins}) == 3
g.shell(
    "gdbus call --system --dest org.freedesktop.Accounts --object-path /org/freedesktop/Accounts/User1000 --method org.freedesktop.Accounts.User.SetSession gnome"
)
logout(logins[-1])
g.shell("systemctl restart gdm")
wait("GNOME fallback", lambda: g.shell("pgrep -u joe -x gnome-shell || :"), 65)
wait(
    "GNOME Wayland environment",
    lambda: "XDG_CURRENT_DESKTOP=GNOME" in as_joe("systemctl --user show-environment"),
)
assert as_joe("systemctl --user is-active mundane-session.target || :") == "inactive"
version = wait(
    "GNOME Shell D-Bus interface",
    lambda: as_joe(
        "busctl --user get-property org.gnome.Shell /org/gnome/Shell org.gnome.Shell ShellVersion 2>/dev/null || :"
    ),
)
assert "51." in version, version
sid = g.shell("loginctl show-seat seat0 -p ActiveSession --value")
assert {"Type=wayland", "Active=yes", "Name=joe"} <= set(
    g.shell(f"loginctl show-session {sid} -p Type -p Active -p Name").splitlines()
)
g.screen("gnome")
nodes = {node["node-name"]: node for node in g.qmp("query-named-block-nodes")}
assert nodes["base"]["ro"] and nodes["basefile"]["ro"] and not nodes["layer"]["ro"]
base = nodes["base"]["image"]["filename"]
assert (
    subprocess.check_output(["stat", "-c", "%s %Y %Z %i", base], text=True)
    == (g.run / "backing.stat").read_text()
)
print(
    f"GDM started GNOME Shell {version} on Wayland for joe. Backing image verified read-only.",
    flush=True,
)
