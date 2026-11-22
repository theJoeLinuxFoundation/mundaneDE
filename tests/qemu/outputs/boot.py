#!/usr/bin/env python3
"""Boot a diskless, headless DRM guest using a local kernel and its matching modules."""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import signal
from driver import serve

root = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--kernel", type=Path, required=True, help="/usr/lib/modules/VERSION/vmlinuz")
parser.add_argument("--binary", type=Path, default=root / "target/debug/mundane-de")
parser.add_argument(
    "--extra-usr", type=Path, help="optional read-only package overlay for guest applications"
)
args = parser.parse_args()
binary = args.binary.resolve(strict=True)
version = args.kernel.resolve(strict=True).parent.name
work = Path(tempfile.mkdtemp(dir=root / "target", prefix="kms-"))
print(work, flush=True)
stage = work / "initrd"
bundle = work / "bundle"
bundle.mkdir()
shutil.copy(binary, bundle / "mundane-de")
shutil.copy(shutil.which("wayland-info"), bundle / "wayland-info")
for name in ("session.sh", "checks.py", "scale_client.py"):
    shutil.copy(Path(__file__).with_name(name), bundle)
for name in ("mundane-files",):
    shutil.copy(shutil.which(name), bundle)
probe = binary.parent / "examples" / "nested-capabilities"
shutil.copy(probe, bundle)
for d in ["usr/bin", "usr/lib", "dev", "proc", "sys", "tmp", "run", "etc", "test"]:
    (stage / d).mkdir(parents=True, exist_ok=True)
for name, target in [
    ("lib", "usr/lib"),
    ("lib64", "usr/lib"),
    ("bin", "usr/bin"),
    ("sbin", "usr/bin"),
]:
    (stage / name).symlink_to(target)


def copy(path):
    dest = stage / str(path).lstrip("/")
    dest.parent.mkdir(exist_ok=True, parents=True)
    shutil.copy2(path, dest)


for name in ["bash", "mount", "modprobe"]:
    path = Path(shutil.which(name)).resolve()
    dest = stage / "usr/bin" / name
    shutil.copy2(path, dest)
    for lib in re.findall(r"(/[^\s()]+)", subprocess.check_output(["ldd", path], text=True)):
        copy(Path(lib))
for mod in ["9p", "9pnet_virtio", "virtio_pci"]:
    for line in subprocess.check_output(
        ["modprobe", "-S", version, "--show-depends", mod], text=True
    ).splitlines():
        if line.startswith("insmod "):
            copy(Path(line.split()[1]))
for f in Path("/usr/lib/modules", version).glob("modules.*"):
    if f.is_file():
        copy(f)
init = stage / "init"
init.write_text("""#!/bin/bash
set -eu
export PATH=/usr/bin
mount -t devtmpfs devtmpfs /dev
exec </dev/console >/dev/console 2>&1
mount -t proc proc /proc
mount -t sysfs sys /sys
modprobe virtio_pci
modprobe 9pnet_virtio
modprobe 9p
mount -t 9p -o trans=virtio,version=9p2000.L,ro,msize=1048576 usr /usr
mount -t 9p -o trans=virtio,version=9p2000.L,ro test /test
exec /bin/bash /test/session.sh
""")
init.chmod(0o755)
with (work / "boot.cpio").open("wb") as archive:
    subprocess.run(
        ["bsdtar", "--uid", "0", "--gid", "0", "--format", "newc", "-cf", "-", "."],
        cwd=stage,
        stdout=archive,
        check=True,
    )
cmd = [
    "qemu-system-x86_64",
    "-enable-kvm",
    "-cpu",
    "host",
    "-m",
    "3072",
    "-smp",
    "4",
    "-nodefaults",
    "-no-reboot",
    "-display",
    "none",
    "-nic",
    "none",
    "-serial",
    "stdio",
    "-kernel",
    str(args.kernel),
    "-initrd",
    str(work / "boot.cpio"),
    "-append",
    "console=ttyS0 quiet panic=-1",
    "-device",
    "virtio-vga,id=gpu,max_outputs=2,xres=800,yres=600",
    "-device",
    "virtio-tablet-pci",
    "-device",
    "virtio-keyboard-pci",
    "-qmp",
    f"unix:{work}/qmp,server=on,wait=off",
]
cmd += [
    "-device",
    "virtio-serial-pci",
    "-chardev",
    f"socket,path={work}/commands,id=testcmd,server=on,wait=off",
    "-device",
    "virtserialport,chardev=testcmd,name=mundane-test",
]
shares = [("usr", "/usr"), ("test", bundle)]
if args.extra_usr:
    shares.append(("extra", args.extra_usr.resolve(strict=True)))
for name, source in shares:
    cmd += ["-virtfs", f"local,path={source},mount_tag={name},security_model=none,readonly=on"]
with (work / "serial.log").open("wb") as log:
    proc = subprocess.Popen(
        cmd,
        stdout=log,
        stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL,
        env={"PATH": os.defpath, "HOME": str(work), "XDG_RUNTIME_DIR": str(work)},
    )
    try:

        def timeout(*_):
            raise TimeoutError("guest test")

        signal.signal(signal.SIGALRM, timeout)
        signal.alarm(180)
        serve(work, proc)
        assert proc.wait(timeout=10) == 0
    finally:
        signal.alarm(0)
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=5)
if "KMS TEST PASSED" not in (work / "serial.log").read_text(errors="replace"):
    raise SystemExit(f"Guest failed. See {work}/serial.log")
