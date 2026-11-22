"""Stage the two standalone agents without installing packages on the host."""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

PACKAGES = {
    "swaylock": (
        "1.8.6-1",
        "4662cc18b71e2875e1d766c63e68d21a9370abb167c0968970c966d5e6044ad5",
        "usr/bin/swaylock",
    ),
    "polkit-gnome": (
        "0.105-12",
        "4413127849dd3b541d8ede73f5ff8d6441037c59a823023e2cddb6da2312a913",
        "usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1",
    ),
}


def agent(name):
    version, digest, member = PACKAGES[name]
    installed = Path("/") / member
    if installed.is_file():
        return installed
    cache = (
        Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
        / "mundane-try"
        / "packages"
    )
    cache.mkdir(exist_ok=True, parents=True)
    filename = f"{name}-{version}-x86_64.pkg.tar.zst"
    pkg = cache / filename
    output = cache / (name + "-" + version + "-agent")
    with tempfile.TemporaryDirectory(dir=cache) as work:
        download = Path(work) / filename
        if not pkg.exists() or hashlib.sha256(pkg.read_bytes()).hexdigest() != digest:
            subprocess.run(
                [
                    "curl",
                    "-fLsS",
                    "--retry",
                    "2",
                    "-o",
                    download,
                    f"https://archive.archlinux.org/packages/{name[0]}/{name}/{filename}",
                ],
                check=True,
            )
            if hashlib.sha256(download.read_bytes()).hexdigest() != digest:
                raise RuntimeError(f"Invalid checksum for {name}")
            os.replace(download, pkg)
        extracted = Path(work) / "agent"
        with extracted.open("wb") as f:
            subprocess.run(["bsdtar", "-xOf", pkg, member], stdout=f, check=True)
        os.replace(extracted, output)
    return output


def stage_shell(share):
    for source, target in [
        (agent("swaylock"), "swaylock"),
        (Path("/usr/lib/libgtk4-layer-shell.so.0"), "libgtk4-layer-shell.so.0"),
        (
            Path("/usr/lib/girepository-1.0/Gtk4LayerShell-1.0.typelib"),
            "Gtk4LayerShell-1.0.typelib",
        ),
    ]:
        shutil.copyfile(source, share / target)


INSTALL_SHELL = """
install -Dm755 /mnt/swaylock /usr/local/bin/swaylock
install -Dm755 /mnt/libgtk4-layer-shell.so.0 /usr/local/lib/libgtk4-layer-shell.so.0
install -Dm644 /mnt/Gtk4LayerShell-1.0.typelib /usr/local/lib/girepository-1.0/Gtk4LayerShell-1.0.typelib
if ! test -e /etc/pam.d/swaylock; then
    printf '%s\\n' 'auth include system-auth' > /etc/pam.d/swaylock
fi
"""
