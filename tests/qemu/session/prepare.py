#!/usr/bin/env python3
"""Install the session into a newly booted joe test overlay and reboot enforcing."""

import argparse
from pathlib import Path
import os
import shutil
import subprocess
import tarfile
import tempfile
import time

from guest import Guest
from dependencies import stage_shell, INSTALL_SHELL

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mundane-de")
parser.add_argument("--libseat", type=Path, required=True)
parser.add_argument(
    "--settings", type=Path, required=True, help="built mundane-settings"
)
parser.add_argument(
    "--refresh", action="store_true", help="update an already prepared overlay"
)
parser.add_argument(
    "--trace-frames",
    action="store_true",
    help="log GPUI qualification timestamps in the guest journal",
)
args = parser.parse_args()
guest = Guest(args.run)
share = guest.run / "share"
nodes = {n["node-name"]: n for n in guest.qmp("query-named-block-nodes")}
assert nodes["base"]["ro"] and nodes["basefile"]["ro"] and not nodes["layer"]["ro"]
for source, name in (
    (args.binary, "mundane-de"),
    (args.libseat, "libseat.so.1"),
    (args.settings, "mundane-settings"),
    *(
        (args.settings.with_name(n), n)
        for n in ("mundane-shell", "mundane-session", "mundane-lock")
    ),
):
    if source and source.resolve() != (share / name).resolve():
        shutil.copyfile(source, share / name)
# Build the installation tree on the host. The guest receives only package data.
with tempfile.TemporaryDirectory(dir=guest.run) as stage:
    subprocess.run(
        [
            ROOT / "session/install.sh",
            share / "mundane-de",
            share,
        ],
        env={**os.environ, "DESTDIR": stage, "PREFIX": "/usr/local"},
        check=True,
    )
    if args.trace_frames:
        entry = Path(stage) / "usr/share/wayland-sessions/mundane.desktop"
        entry.write_text(
            entry.read_text().replace(
                "\nExec=/usr/local/bin/mundane-session",
                "\nExec=/usr/bin/env RUST_LOG=info,mundane_shell_frames=debug /usr/local/bin/mundane-session",
            )
        )
    with tarfile.open(share / "session.tar", "w") as archive:
        archive.add(Path(stage) / "usr", arcname="usr")
for name in ("probe.py", "shell_widgets.py", "org.joelinux.MundaneSessionTest.service"):
    shutil.copyfile(Path(__file__).with_name(name), share / name)


stage_shell(share)


def prompt(offset=0):
    log = guest.run / "serial.log"
    for _ in range(300):
        with log.open("rb") as stream:
            stream.seek(offset)
            if b"joe# " in stream.read():
                return
        time.sleep(1)
    raise RuntimeError("joe-test-console prompt did not arrive")


prompt()
print(
    guest.shell(
        """
set -eu
stty -echo cols 250
dmesg -n 1
. /etc/os-release
test "$ID" = joe
test -f /usr/lib/dave/sealed
"""
        + (
            "systemctl stop gdm"
            if args.refresh
            else 'test "$(cat /sys/kernel/security/joe/enforce)" = 0'
        )
        + """
mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt
tar -C / -xf /mnt/session.tar --no-same-owner
install -Dm755 /mnt/libseat.so.1 /usr/local/lib/libseat.so.1
"""
        + INSTALL_SHELL
        + """
ldconfig
# These names belonged to earlier versions of this harness on persistent overlays.
rm -f /usr/local/bin/mundane-panel /usr/share/wayland-sessions/mundane-gpui.desktop
rm -rf /usr/local/share/mundane/shell
rm -f /usr/local/share/applications/mundane-gpui-mimeapps.list /usr/local/share/xdg-desktop-portal/mundane-gpui-portals.conf
# Remove helper copies staged by earlier harness revisions, when still identical.
for name in swaylock libgtk4-layer-shell.so.0 Gtk4LayerShell-1.0.typelib; do
    case $name in
        swaylock) path=/usr/local/bin/$name ;;
        *.typelib) path=/usr/local/lib/girepository-1.0/$name ;;
        *) path=/usr/local/lib/$name ;;
    esac
    if test -f "/mnt/$name" && cmp -s "/mnt/$name" "$path"; then rm -f "$path"; fi
done
# beta1's portal frontend also searches its compiled-in data directory.
install -Dm644 /usr/local/share/xdg-desktop-portal/mundane-portals.conf /usr/share/xdg-desktop-portal/mundane-portals.conf
install -Dm644 /usr/local/share/xdg-desktop-portal/portals/mundane.portal /usr/share/xdg-desktop-portal/portals/mundane.portal

install -Dm644 /mnt/shell_widgets.py /usr/local/libexec/mundane-test-widgets
install -Dm755 /mnt/probe.py /usr/local/libexec/mundane-login-probe
install -Dm644 /mnt/org.joelinux.MundaneSessionTest.service /usr/local/share/dbus-1/services/org.joelinux.MundaneSessionTest.service
test -x /usr/local/bin/mundane-de
"""
        + """
session=mundane
for method in SetSession:$session SetSessionType:wayland; do
    gdbus call --system --dest org.freedesktop.Accounts --object-path /org/freedesktop/Accounts/User1000 --method "org.freedesktop.Accounts.User.${method%:*}" "${method#*:}"
done
umount /mnt
grep -q '^options .*joe=enforce' /efi/loader/entries/joe-1.conf || sed -i '/^options /s/$/ joe=enforce/' /efi/loader/entries/joe-1.conf
sync
echo "$PRETTY_NAME"
"""
    ),
    flush=True,
)
offset = (guest.run / "serial.log").stat().st_size
guest.shell("systemd-run --on-active=1s --unit=mundane-test-reboot systemctl reboot")
# A prompt just before the reboot can also be present. Check the new boot.
for _ in range(180):
    if (
        b"joe login: root (automatic login)"
        in (guest.run / "serial.log").read_bytes()[offset:]
    ):
        prompt((guest.run / "serial.log").read_bytes().rindex(b"joe login: root"))
        break
    time.sleep(1)
else:
    raise RuntimeError("enforcing boot did not reach the test console")
print(
    guest.shell(
        'stty -echo cols 250; dmesg -n 1; test "$(cat /sys/kernel/security/joe/enforce)" = 1 && echo enforcing'
    ),
    flush=True,
)
