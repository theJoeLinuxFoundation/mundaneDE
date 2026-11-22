#!/usr/bin/env python3
"""Install the session into a newly booted joe test overlay and reboot enforcing."""

import argparse
from pathlib import Path
import shutil
import time

from guest import Guest

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mundane-de")
parser.add_argument("--libseat", type=Path, required=True)
parser.add_argument("--agent", type=Path, required=True, help="standalone graphical polkit agent")
parser.add_argument("--settings", type=Path, required=True, help="built mundane-settings")
args = parser.parse_args()
guest = Guest(args.run)
share = guest.run / "share"
nodes = {n["node-name"]: n for n in guest.qmp("query-named-block-nodes")}
assert nodes["base"]["ro"] and nodes["basefile"]["ro"] and not nodes["layer"]["ro"]
for source, name in (
    (args.binary, "mundane-de"),
    (args.libseat, "libseat.so.1"),
    (args.agent, "mundane-polkit-agent"),
    (args.settings, "mundane-settings"),
):
    if source and source.resolve() != (share / name).resolve():
        shutil.copyfile(source, share / name)
shutil.copytree(ROOT / "session", share / "session", dirs_exist_ok=True)
for name in ("probe.py", "org.joelinux.MundaneSessionTest.service"):
    shutil.copyfile(Path(__file__).with_name(name), share / name)


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
    guest.shell("""
set -eu
stty -echo cols 250
dmesg -n 1
. /etc/os-release
test "$ID" = joe
test -f /usr/lib/dave/sealed
test "$(cat /sys/kernel/security/joe/enforce)" = 0
mount -t 9p -o trans=virtio,version=9p2000.L,ro mundane /mnt
sh /mnt/session/install.sh /mnt/mundane-de
install -Dm755 /mnt/libseat.so.1 /usr/local/lib/libseat.so.1
install -Dm755 /mnt/mundane-polkit-agent /usr/local/bin/mundane-polkit-agent
ldconfig
install -Dm755 /mnt/mundane-settings /usr/local/bin/mundane-settings
install -Dm755 /mnt/probe.py /usr/local/libexec/mundane-login-probe
install -Dm644 /mnt/org.joelinux.MundaneSessionTest.service /usr/local/share/dbus-1/services/org.joelinux.MundaneSessionTest.service
/usr/local/bin/mundane-de --help >/dev/null
for method in SetSession:mundane SetSessionType:wayland; do
    gdbus call --system --dest org.freedesktop.Accounts --object-path /org/freedesktop/Accounts/User1000 --method "org.freedesktop.Accounts.User.${method%:*}" "${method#*:}"
done
umount /mnt
sed -i '/^options /s/$/ joe=enforce/' /efi/loader/entries/joe-1.conf
sync
echo "$PRETTY_NAME, original GNOME session $(pgrep -u joe -x gnome-shell)"
"""),
    flush=True,
)
offset = (guest.run / "serial.log").stat().st_size
guest.shell("systemctl reboot --no-block")
# A prompt just before the reboot can also be present. Check the new boot.
for _ in range(180):
    if b"joe login: root (automatic login)" in (guest.run / "serial.log").read_bytes()[offset:]:
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
