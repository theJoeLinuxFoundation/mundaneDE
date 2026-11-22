"""Stage the session helpers in a private overlay without installing them on the host."""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tarfile

PACKAGES = {
    "grim": (
        "1.5.0-2",
        "2cbddb88d4a3316c61e8984203bbdca81cbf089c323e5db09c4e1648c483c8a6",
        "usr/bin/grim",
    ),
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


def screencast_backend():
    # 0.9.0 sequences capture after PipeWire starts and returns the node serial.
    revision = "8388b8a6214470462146840e92909aff5983eeac"
    digest = "f7f3c4ca43525e41bb0caaf6ae39ba607a77624bff345d90b9bd2e4d6109569c"
    cache = (
        Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
        / "mundane-try/packages"
    )
    cache.mkdir(exist_ok=True, parents=True)
    binary = cache / ("xdg-desktop-portal-wlr-" + revision)
    if binary.is_file():
        return binary
    meson = ["meson"]
    if not (shutil.which("meson") and shutil.which("ninja")):
        meson = ["uvx", "--with", "ninja==1.13.0", "--from", "meson==1.10.0", "meson"]
    with tempfile.TemporaryDirectory(dir=cache) as tmp:
        work = Path(tmp)
        src = work / ("xdg-desktop-portal-wlr-" + revision)
        archive = work / "source.tar.gz"
        subprocess.run(
            [
                "curl",
                "-fLsS",
                "--retry",
                "2",
                "-o",
                archive,
                "https://codeload.github.com/emersion/xdg-desktop-portal-wlr/tar.gz/"
                + revision,
            ],
            check=True,
        )
        if hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
            raise RuntimeError("Invalid xdg-desktop-portal-wlr source checksum")
        with tarfile.open(archive) as tar:
            tar.extractall(work, filter="data")
        build = work / "build"
        subprocess.run(
            [
                *meson,
                "setup",
                build,
                src,
                "-Dman-pages=disabled",
                "-Dsystemd=disabled",
                "--buildtype=release",
            ],
            check=True,
        )
        subprocess.run([*meson, "compile", "-C", build], check=True)
        os.replace(build / "xdg-desktop-portal-wlr", binary)
    return binary


def stage_shell(share):
    for source, target in [
        (agent("swaylock"), "swaylock"),
        (screencast_backend(), "xdg-desktop-portal-wlr"),
        (agent("grim"), "grim"),
        (Path("/usr/lib/libjpeg.so.8"), "libjpeg.so.8"),
        (Path("/usr/lib/libgtk4-layer-shell.so.0"), "libgtk4-layer-shell.so.0"),
        (
            Path("/usr/lib/girepository-1.0/Gtk4LayerShell-1.0.typelib"),
            "Gtk4LayerShell-1.0.typelib",
        ),
    ]:
        shutil.copyfile(source, share / target)


INSTALL_SHELL = """
install -Dm755 /mnt/xdg-desktop-portal-wlr /usr/local/bin/xdg-desktop-portal-wlr
install -Dm755 /mnt/grim /usr/local/bin/grim
install -Dm755 /mnt/libjpeg.so.8 /usr/local/lib/libjpeg.so.8
install -Dm755 /mnt/swaylock /usr/local/bin/swaylock
install -Dm755 /mnt/libgtk4-layer-shell.so.0 /usr/local/lib/libgtk4-layer-shell.so.0
install -Dm644 /mnt/Gtk4LayerShell-1.0.typelib /usr/local/lib/girepository-1.0/Gtk4LayerShell-1.0.typelib
if ! test -e /etc/pam.d/swaylock; then
    printf '%s\\n' 'auth include system-auth' > /etc/pam.d/swaylock
fi
"""
