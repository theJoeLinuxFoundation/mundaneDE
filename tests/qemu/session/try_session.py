"""Boot a persistent GDM hand-test guest. --headless uses exactly the same disk and session."""

import argparse
import fcntl
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import time

from guest import Guest
from machine import boot

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument(
    "--headless", action="store_true", help="use QMP only and leave the guest running"
)
parser.add_argument(
    "--image",
    type=Path,
    default=Path.home() / "Projects/joe-linux/build/images/joe-desktop-beta1.img",
)
parser.add_argument("--source", type=Path, default=Path(__file__).resolve().parents[3])
# Older launchers run this file directly inside a git archive.
parser.add_argument("--build-label", default="$Format:%H$")
args = parser.parse_args()
if args.build_label.startswith("$"):
    # Direct callers in a checkout use the same publication checks as the shell entry point.
    driver = str(Path(__file__).with_name("try_revision.py"))
    os.execv(sys.executable, [sys.executable, driver, *sys.argv[1:]])
ROOT = args.source.resolve(strict=True)
fixtures = ROOT / "tests/qemu/session"
title = "mundaneDE on joe Linux — " + args.build_label
cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
run = Path(
    os.environ.get(
        "MUNDANE_TRY_DIR",
        cache / "mundane-try" / ("qualification" if args.headless else "desktop"),
    )
).resolve()
if args.headless and any(
    run.is_relative_to(cache.resolve() / "mundane-try" / name)
    for name in ("desktop", "hand-test")
):
    parser.error("choose a dedicated directory for headless tests")
run.mkdir(mode=0o700, exist_ok=True, parents=True)
guard = (run / "launcher.lock").open("a")
fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
g = Guest(run)


def running():
    try:
        return g.qmp("query-status") is not None
    except (FileNotFoundError, ConnectionRefusedError):
        return False


def shutdown():
    try:
        g.shell(
            "systemd-run --on-active=1s --unit=mundane-hand-test-poweroff systemctl poweroff",
            timeout=10,
        )
    except (TimeoutError, EOFError, ConnectionError):
        pass
    for _ in range(120):
        if not running():
            return
        time.sleep(0.5)
    raise RuntimeError("Guest still shutting down. Its overlay is in use.")


def ready():
    for _ in range(120):
        if b"joe# " in (run / "serial.log").read_bytes():
            if (
                g.shell(
                    "sid=$(loginctl show-seat seat0 -p ActiveSession --value); "
                    'test -n "$sid" && test "$(loginctl show-session "$sid" -p Name --value)" = joe '
                    "&& pgrep -u joe -f '^/usr/local/bin/mundane-de --session-notify' >/dev/null "
                    "&& su joe -c 'busctl --address=unix:path=/run/user/1000/bus --timeout=2 call "
                    "org.kde.StatusNotifierWatcher /StatusNotifierWatcher org.freedesktop.DBus.Peer Ping' "
                    ">/dev/null 2>&1 && echo ready || :"
                )
                == "ready"
            ):
                return
        time.sleep(1)
    raise RuntimeError(f"GDM did not start the shell. See {run}/serial.log")


if running():
    sys.exit(f"The guest in {run} is already running. Shut it down before relaunching.")
image = args.image.resolve(strict=True)
subprocess.run(
    ["cargo", "+stable", "build", "--locked", "--release", "--bin", "mundane-de"],
    cwd=ROOT,
    check=True,
)
subprocess.run(
    [
        "cargo",
        "+stable",
        "build",
        "--locked",
        "--manifest-path",
        "settings-panel/Cargo.toml",
    ],
    cwd=ROOT,
    check=True,
)
settings = run / "mundane-settings"
subprocess.run(
    [
        "objcopy",
        "--strip-debug",
        ROOT / "settings-panel/target/debug/mundane-settings",
        settings,
    ],
    check=True,
)
binary = ROOT / "target/release/mundane-de"
source_hash = hashlib.sha256()
for path in [
    binary,
    settings,
    *sorted((ROOT / "session").rglob("*")),
    *(
        fixtures / n
        for n in (
            "prepare.py",
            "dependencies.py",
            "probe.py",
            "shell_widgets.py",
            "org.joelinux.MundaneSessionTest.service",
        )
    ),
]:
    if path.is_file() and "__pycache__" not in path.parts:
        source_hash.update(path.name.encode() + b"\0")
        with path.open("rb") as src:
            source_hash.update(hashlib.file_digest(src, "sha256").digest())
revision = source_hash.hexdigest()
stamp = run / "installed.sha256"
install = not stamp.exists() or stamp.read_text() != revision
print(f"Guest files in {run}", flush=True)
if install:
    # Always stage the current checkout in an invisible boot, including on the first run.
    existing = (run / "system.qcow2").exists()
    boot(image, run, large=True, title=title)
    try:
        prepare = [
            sys.executable,
            str(fixtures / "prepare.py"),
            str(run),
            "--binary",
            str(binary),
            "--libseat",
            "/usr/lib/libseat.so.1",
            "--settings",
            str(settings),
        ]
        if existing:
            for _ in range(300):
                if b"joe# " in (run / "serial.log").read_bytes():
                    break
                time.sleep(1)
            # Interrupted initial setup can still be in the first, permissive boot.
            if g.shell("cat /sys/kernel/security/joe/enforce") == "1":
                prepare.append("--refresh")
        subprocess.run(prepare, check=True)
        ready()
        stamp.write_text(revision)
    except BaseException:
        if running():
            g.qmp("quit")
        raise
    if not args.headless:
        shutdown()
visible_guest = None
if not install or not args.headless:
    visible_guest = boot(
        image,
        run,
        display="none" if args.headless else "gtk,show-menubar=off",
        large=True,
        title=title,
    )
    ready()
g.screen("desktop")
print(
    "mundaneDE is running. Use Start on the bottom taskbar or press Super+Space. The test account is joe, password joe.",
    flush=True,
)

if visible_guest is not None:
    sys.exit(visible_guest.wait())
