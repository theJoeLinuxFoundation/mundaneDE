"""Launch the joe guest with an explicitly read-only raw backing chain."""

import json
import os
from pathlib import Path
import shutil
import subprocess


def boot(image, run, display="none", large=False):
    image = Path(image).resolve(strict=True)
    run = Path(run).resolve()
    firmware = Path(os.environ.get("OVMF_DIR", "/usr/share/edk2/x64"))
    run.mkdir(mode=0o700, parents=True, exist_ok=True)
    (run / "share").mkdir(exist_ok=True)
    stamp = subprocess.check_output(["stat", "-c", "%s %Y %Z %i", image])
    expected = run / "backing.stat"
    if not (run / "system.qcow2").exists():
        subprocess.run(
            [
                "qemu-img",
                "create",
                "-q",
                "-f",
                "qcow2",
                "-F",
                "raw",
                "-b",
                image,
                run / "system.qcow2",
            ],
            check=True,
        )
        expected.write_bytes(stamp)
        (run / "backing.path").write_text(str(image))
        shutil.copyfile(firmware / "OVMF_VARS.4m.fd", run / "vars.fd")
    elif expected.read_bytes() != stamp or (run / "backing.path").read_text() != str(
        image
    ):
        raise RuntimeError(
            "Backing image changed. Choose a new MUNDANE_TRY_DIR for this image."
        )
    env = {"PATH": os.environ["PATH"], "HOME": str(run)}
    if display != "none":
        for key in (
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "XAUTHORITY",
            "LANG",
        ):
            if key in os.environ:
                env[key] = os.environ[key]
    card = (
        "virtio-vga,id=gpu,max_outputs=3,xres=1920,yres=1080" if large else "virtio-vga"
    )
    cmd = [
        "qemu-system-x86_64",
        *(["-daemonize"] if display == "none" else []),
        "-pidfile",
        "vm.pid",
        "-name",
        "mundaneDE on joe Linux",
        "-machine",
        "q35,accel=kvm",
        "-cpu",
        "host",
        "-m",
        "6G",
        "-smp",
        "8",
        "-global",
        "ICH9-LPC.disable_s3=0",
        "-nodefaults",
        "-display",
        display,
        "-nic",
        "none",
        "-monitor",
        "none",
        "-device",
        card,
        "-device",
        "virtio-keyboard-pci",
        "-device",
        "virtio-tablet-pci",
        "-drive",
        f"if=pflash,format=raw,readonly=on,file={firmware}/OVMF_CODE.4m.fd",
        "-drive",
        "if=pflash,format=raw,file=vars.fd",
        "-blockdev",
        json.dumps(
            {
                "driver": "file",
                "node-name": "basefile",
                "read-only": True,
                "filename": str(image),
            }
        ),
        "-blockdev",
        "driver=raw,node-name=base,read-only=on,file=basefile",
        "-blockdev",
        "driver=file,node-name=layerfile,filename=system.qcow2",
        "-blockdev",
        "driver=qcow2,node-name=layer,backing=base,file=layerfile",
        "-device",
        "virtio-blk-pci,drive=layer",
        "-virtfs",
        "local,path=share,mount_tag=mundane,security_model=none,readonly=on",
        "-chardev",
        "socket,id=console,path=serial,server=on,wait=off,logfile=serial.log",
        "-serial",
        "chardev:console",
        "-qmp",
        "unix:qmp,server=on,wait=off",
    ]
    if display == "none":
        subprocess.run(cmd, env=env, cwd=run, check=True)
        return None
    return subprocess.Popen(cmd, env=env, cwd=run)
