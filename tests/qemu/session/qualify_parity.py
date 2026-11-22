"""Compare daily shell workflows on the baseline and revised 1920x1080 joe guest."""

import argparse
import shlex
import shutil
import subprocess
import time
from pathlib import Path

from guest import Guest
from qualify_menus import wait


def qualify(run, output, baseline):
    g = Guest(run)
    assert g.qmp("query-display-options")["type"] == "none"
    assert {n["node-name"] for n in g.qmp("query-named-block-nodes") if n["ro"]} >= {
        "base",
        "basefile",
    }
    assert g.shell("cat /sys/kernel/security/joe/enforce") == "1"
    output.mkdir(parents=True, exist_ok=True)

    shutil.copyfile(
        Path(__file__).with_name("shell_widgets.py"), g.run / "share/parity-widgets.py"
    )
    g.shell(
        "mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt && "
        "cp /mnt/parity-widgets.py /tmp/parity-widgets.py && umount /mnt"
    )

    def joe(command):
        return g.shell(
            "su joe -s /bin/sh -c "
            + shlex.quote(
                "export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; "
                "export WAYLAND_DISPLAY=$(systemctl --user show-environment | sed -n 's/^WAYLAND_DISPLAY=//p'); "
                + command
            )
        )

    def widget(verb, name=""):
        return joe(
            "python3 -W ignore /tmp/parity-widgets.py " + shlex.join([verb, name])
        )

    def has(row):
        return row in widget("list").splitlines()

    def key(*names):
        g.key(*names)
        time.sleep(0.16)

    def held(name, down):
        g.qmp(
            "input-send-event",
            events=[
                {
                    "type": "key",
                    "data": {"down": down, "key": {"type": "qcode", "data": name}},
                }
            ],
        )

    def shot(name):
        time.sleep(0.35)
        ppm = g.screen("parity-" + name)
        assert ppm.startswith(b"P6\n1920 1080\n255\n")
        path = output / (("before-" if baseline else "after-") + name + ".png")
        with path.open("wb") as png:
            subprocess.run(["pnmtopng"], input=ppm, stdout=png, check=True)

    def active(name):
        return "active" in widget("frame-state", name).split()

    def editor(title):
        return next(
            (
                s.removeprefix("frame ")
                for s in widget("list").splitlines()
                if s.startswith("frame " + title) and "Text Editor" in s
            ),
            "",
        )

    def launch_editor(document):
        # Isolate the editor's saved drafts so a previous guest run cannot alter
        # the window list or put a recovery banner in a comparison screenshot.
        profile = shlex.quote(joe("mktemp -d"))
        joe(
            f"XDG_DATA_HOME={profile}/data XDG_STATE_HOME={profile}/state "
            f"XDG_CACHE_HOME={profile}/cache gnome-text-editor --standalone /tmp/{document} "
            f"</dev/null >{profile}/editor.log 2>&1 &"
        )

    wait("taskbar", lambda: has("button Start"))
    # Use a fresh login with a clear workspace for matching captures.
    assert not has("frame Calculator") and not editor("Week")
    key("esc")
    shot("taskbar")
    key("meta_l", "spc")
    wait("Start", lambda: has("frame Start menu"))
    shot("start")
    key("esc")
    if baseline:
        joe(
            "gio launch /usr/share/applications/org.gnome.Calculator.desktop >/dev/null 2>&1"
        )
    else:
        widget("click", "Calculator")
    wait("calculator window", lambda: has("frame Calculator") and active("Calculator"))
    joe(
        "printf 'Monday\n\nReview the desktop package\nCheck the hand-test notes\n' >/tmp/Week.txt; "
    )
    launch_editor("Week.txt")
    wait("editor focus", lambda: (name := editor("Week")) and active(name))
    held("alt", True)
    try:
        key("tab")
        if not baseline:
            wait("switcher", lambda: has("frame Switch windows"))
            wait(
                "two windows in switcher",
                lambda: has("label 2 of 2  ·  Release Alt to switch"),
            )
        shot("switcher")
    finally:
        held("alt", False)
    if baseline:
        assert not has("frame Switch windows")
        return
    wait("switcher closed", lambda: not has("frame Switch windows"))
    wait("release selects calculator", lambda: active("Calculator"))
    key("alt", "tab")
    wait("quick Alt+Tab returns to editor", lambda: active(editor("Week")))
    held("alt", True)
    try:
        key("shift", "tab")
        wait("reverse switcher", lambda: has("frame Switch windows"))
        key("esc")
    finally:
        held("alt", False)
    assert not active("Calculator") and not has("frame Switch windows")
    print("Alt+Tab preview, release, quick return and Escape passed", flush=True)

    # A running pin activates its existing window; two windows offer a named choice.
    widget("click", "Calculator")
    wait("running pin focuses same calculator", lambda: active("Calculator"))
    assert widget("list").splitlines().count("frame Calculator") == 1
    joe("printf 'Tuesday\n' > /tmp/Travel.txt; ")
    launch_editor("Travel.txt")
    wait("second document", lambda: (name := editor("Travel")) and active(name))
    widget("click", "Text Editor")

    def document_button():
        return next(
            (
                s.removeprefix("button ")
                for s in widget("list").splitlines()
                if s.startswith("button Week.txt") and "Desk 1" in s
            ),
            "",
        )

    wait("group lists named windows", document_button)
    widget("click", document_button())
    wait("chosen document focused", lambda: active(editor("Week")))
    held("alt", True)
    try:
        key("shift", "tab")
        wait(
            "backward traversal", lambda: has("label 3 of 3  ·  Release Alt to switch")
        )
    finally:
        held("alt", False)
    wait("reverse skips previous document", lambda: active("Calculator"))

    # A minimized calculator remains reachable. An empty desk has no candidates.
    widget("click", "Minimize")
    wait("calculator minimized", lambda: not active("Calculator"))
    widget("click", "Calculator")
    wait("pin restores minimized calculator", lambda: active("Calculator"))
    key("meta_l", "2")
    wait("empty workspace", lambda: has("button Workspaces, current 2"))
    held("alt", True)
    try:
        key("tab")
        assert not has("frame Switch windows")
    finally:
        held("alt", False)
    key("meta_l", "1")
    key("meta_l", "spc")
    widget("click", "Unpin Calculator")
    wait("removed from favourites", lambda: not has("button Unpin Calculator"))
    key("esc")
    # Pin by a desktop-entry keyword that does not occur in the display name.
    key("meta_l", "spc")
    for letter in "arithmetic":
        key(letter)
    wait("keyword finds Calculator", lambda: has("button Pin Calculator"))
    widget("click", "Pin Calculator")
    wait("pin saved", lambda: has("button Unpin Calculator"))
    key("esc")
    key("meta_l", "spc")
    key("tab")
    key("spc")
    wait("browse application categories", lambda: has("button Utilities"))
    widget("click", "Utilities")
    wait(
        "Utilities category",
        lambda: (
            has("button Unpin Calculator")
            and has("button Pin Disks")
            and not has("button Unpin Web Browser")
        ),
    )
    key("esc")
    key("meta_l", "spc")
    widget("click", "Unpin Files")
    wait("Files removed from favourites", lambda: not has("button Unpin Files"))
    key("esc")
    wait("Files pin removed from the bar", lambda: not has("button Files"))
    pid = g.shell("pgrep -u joe -f '/mundane/shell/panel.py' | head -1")
    g.shell("kill " + str(int(pid)))
    wait(
        "new panel",
        lambda: (
            (p := g.shell("pgrep -u joe -f '/mundane/shell/panel.py' | head -1"))
            and p != pid
        ),
    )
    wait("pins survive panel restart", lambda: has("button Calculator"))
    key("meta_l", "spc")
    wait("persisted pin in favourites", lambda: has("button Unpin Calculator"))
    assert not has("button Unpin Files")
    key("esc")
    joe("gsettings reset org.joelinux.mundane.shell favorite-apps")
    wait("inherited Files pin restored", lambda: has("button Files"))
    print(
        "Pin launch, activation, grouped selection, reverse traversal, keyword search, category and persistence passed"
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    p.add_argument("output", type=Path)
    p.add_argument("--baseline", action="store_true")
    a = p.parse_args()
    qualify(a.run, a.output, a.baseline)
