#!/usr/bin/env python3
"""Run a real compositor and client on a private, in-memory Wayland output."""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = "HEADLESS-1"
GREEN = bytes.fromhex("20df61")
LANDMARKS = [
    (0.10, 0.15, (82, 79, 98)),
    (0.30, 0.50, (188, 143, 137)),
    (0.695, 0.375, (255, 220, 178)),
    (0.80, 0.63, (118, 105, 125)),
    (0.50, 0.90, (57, 80, 96)),
]


def wait_for(description, probe, processes=()):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError(f"{process.args[0]} exited with {process.returncode} during {description}")
        result = probe()
        if result:
            return result
        time.sleep(0.08)
    raise TimeoutError(description)


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def run(args, env):
    return subprocess.check_output(args, env=env, timeout=6)


def screenshot(env, path):
    ppm = run(["grim", "-o", OUTPUT, "-t", "ppm", "-"], env)
    path.write_bytes(ppm)
    header = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", ppm)
    assert header, "grim did not return an RGB PPM"
    width, height = map(int, header.groups())
    pixels = ppm[header.end():]
    assert len(pixels) == width * height * 3
    return width, height, pixels


def wallpaper(frame, expected_size):
    width, height, pixels = frame
    if (width, height) != expected_size:
        return False
    # Five interior landmarks in the 16:9 Afterlight image, away from ridge edges.
    for x, y, rgb in LANDMARKS:
        offset = 3 * (int(y * height) * width + int(x * width))
        if max(abs(a - b) for a, b in zip(rgb, pixels[offset:offset + 3])) > 6:
            return False
    return True


def client_frame(frame, requested):
    width, _, pixels = frame
    # The fixture contains one opaque green rectangle, with no decorations.
    first = pixels.find(GREEN)
    if first < 0 or first % 3:
        return False
    x = (first // 3) % width
    w, h = requested
    if x + w > width or pixels.count(GREEN) != w * h:
        return False
    row = GREEN * w
    return all(pixels.startswith(row, first + y * width * 3) for y in range(h))


def scenario(base, binary, gpu, env, everyday, frame_rate):
    if frame_rate:
        cfg = Path(env["XDG_CONFIG_HOME"]) / "mundane"
        cfg.mkdir()
        (cfg / "config.toml").write_text(f"[output]\nrefresh={frame_rate*1000}\n")
        env["RUST_LOG"] = "info,smithay::backend::winit=debug,mundane_frame=trace,mundane_de::backend::winit=trace"
    host_log = base / "sway.log"
    server_log = base / "mundane.log"
    trace = base / "client.log"
    config = base / "sway.conf"
    config.write_text("swaybg_command -\nxwayland disable\ndefault_border none\n"
                      "seat seat0 fallback true\noutput HEADLESS-1 mode 640x360\n")
    runtime = Path(env["XDG_RUNTIME_DIR"])
    host_env = dict(env, WLR_BACKENDS="headless", WLR_HEADLESS_OUTPUTS="1",
                    WLR_RENDERER="gles2" if gpu else "pixman")
    if gpu:
        host_env["WLR_RENDER_DRM_DEVICE"] = gpu
    with ExitStack() as cleanup:
        def launch(argv, log, child_env, **kwargs):
            stream = cleanup.enter_context(log.open("w"))
            child = subprocess.Popen(argv, env=child_env, stdout=stream,
                                     stderr=subprocess.STDOUT, **kwargs)
            cleanup.callback(stop, child)
            return child

        bus = launch(["dbus-daemon", "--nofork", "--config-file",
                      str(Path(__file__).with_name("bus.conf"))], base / "bus.log", env)
        wait_for("private bus", (runtime / "bus").is_socket, [bus])
        host = launch(["sway", "-d", "--unsupported-gpu", "-c", str(config)], host_log, host_env)
        ipc = wait_for("private Sway IPC", lambda: next(runtime.glob(f"sway-ipc.*.{host.pid}.sock"), None), [host])
        # Discover only sockets inside our initially empty runtime directory.
        host_socket = wait_for("private Wayland socket",
                               lambda: next((p for p in runtime.glob("wayland-*") if p.is_socket()), None), [host])
        host_client = dict(env, WAYLAND_DISPLAY=host_socket.name)

        def swaymsg(*cmd):
            return json.loads(run(["swaymsg", "-s", str(ipc), "-r", *cmd], host_client))

        if everyday:
            from everyday import prepare_ibus
            prepare_ibus(base, host_client, launch)
        server = launch([binary, "--winit"], server_log, dict(host_client, WAYLAND_DEBUG="client") if everyday else host_client)


        def ready():
            return re.search(r"winit backend running on WAYLAND_DISPLAY=(wayland-\d+)", server_log.read_text())

        socket = wait_for("nested compositor readiness", ready, [host, server])[1]
        assert socket != host_socket.name and (runtime / socket).is_socket()
        inner = dict(env, WAYLAND_DISPLAY=socket)
        registry = run(["wayland-info"], inner).decode()
        (base / "registry.txt").write_text(registry)
        globals_found = sorted(re.findall(r"^interface: '([^']+)'.*?version:\s+(\d+)", registry, re.M))
        baseline = Path(__file__).with_name("registry.txt").read_text().splitlines()
        if gpu:
            baseline = [line.replace("zwp_linux_dmabuf_v1 3", "zwp_linux_dmabuf_v1 5") for line in baseline]
        assert [f"{name} {version}" for name, version in globals_found] == baseline, "ordinary client registry changed"

        def capture(label):
            return screenshot(host_client, base / f"{label}.ppm")

        # Exercise the host-driven winit resize as well as client buffer resizes.
        for w, h in [(640, 360), (800, 450)]:
            if w != 640:
                assert swaymsg("output", OUTPUT, "mode", f"{w}x{h}")[0]["success"]
            wait_for(f"Afterlight at {w}x{h}", lambda: wallpaper(capture(f"wallpaper-{w}"), (w, h)), [server])

        # Save a fully settled background. Exact equality after client exit detects stale damage.
        def settled():
            frame = capture("settled")
            if wallpaper(frame, (800, 450)):
                time.sleep(0.15)
                if capture("settled") == frame:
                    return frame

        background = wait_for("settled wallpaper", settled, [server])
        app_env = dict(inner, GDK_BACKEND="wayland", GSK_RENDERER="cairo", WAYLAND_DEBUG="client",
                       GTK_A11Y="none")
        client = launch([sys.executable, str(Path(__file__).with_name("client.py"))], trace, app_env,
                        stdin=subprocess.PIPE, text=True)
        wait_for("120x80 mapped client", lambda: client_frame(capture("mapped"), (120, 80)), [server, client])
        client.stdin.write("resize\n")
        client.stdin.flush()
        wait_for("200x110 resized client", lambda: client_frame(capture("resized"), (200, 110)), [server, client])
        client.stdin.write("close\n")
        client.stdin.close()
        assert client.wait(timeout=5) == 0
        wait_for("exact wallpaper restoration", lambda: capture("closed") == background, [server])
        protocol = trace.read_text()
        configure = re.search(r"xdg_surface[@#]\d+\.configure\((\d+)\)", protocol)
        assert configure and re.search(rf"ack_configure\({configure[1]}\)", protocol[configure.end():])
        for w, h in [(120, 80), (200, 110)]:
            assert re.search(rf"create_buffer\([^\n]+, {w}, {h},", protocol), f"missing {w}x{h} shm buffer"
        probe = binary.parent / "examples/nested-capabilities"
        if probe.is_file():
            result = run([probe, runtime / socket, "--presentation"],inner).decode()
            assert "superseded commit discarded" in result
            (base / "presentation.log").write_text(result)
        if gpu:
            egl_trace = base / "egl-client.log"
            egl_app = launch([sys.executable, str(Path(__file__).with_name("client.py"))], egl_trace,
                             dict(app_env,GSK_RENDERER="gl"),stdin=subprocess.PIPE,text=True)
            wait_for("EGL client matches shm reference",lambda:client_frame(capture("egl"),(120,80)),[egl_app])
            assert "create_immed(" in egl_trace.read_text() or ".created(" in egl_trace.read_text(), "expected a DMA-BUF-backed EGL client"
            egl_app.stdin.write("close\n")
            egl_app.stdin.close()
            assert egl_app.wait(timeout=5)==0
        if frame_rate:
            from budget import measure
            measure(base,binary,inner,host_client,launch,swaymsg,frame_rate)
        if everyday:
            from everyday import exercise
            exercise(base, binary, inner, host_client, launch, capture, background, server.pid)
        assert swaymsg(f"[pid={server.pid}]", "kill")[0]["success"]
        assert server.wait(timeout=5) == 0
        log = server_log.read_text()
        assert not re.search(r"\[EGL\] 0x|Failed to query buffer age|render error:", log), "renderer error (see mundane.log)"
        assert "queried nested EGL buffer age" in log, "buffer-age tracking never resumed"
        renderer = re.search(r'GL Renderer: "([^"]+)"', log)[1]
        assert ("NVIDIA" if gpu else "llvmpipe") in renderer, f"unexpected renderer {renderer}"
        print(f"Passed on {renderer}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gpu", metavar="RENDER_NODE", help="test an isolated NVIDIA GBM output")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mundane-de")
    parser.add_argument("--everyday", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--frame-rate",type=int,choices=[60,120],help="run the one-minute release frame budget workload")
    args = parser.parse_args()
    if args.everyday and (Path("/proc/1/comm").read_text().strip() != "bwrap" or Path("/run/user").exists() or Path("/dev/dri").exists()
                          or Path("/sys/class/net").exists()):
        parser.error("use everyday.sh to create the private X11, network and device namespaces")
    if not __debug__:
        parser.error("run without -O so the test assertions are enabled")
    if not args.binary.is_file():
        parser.error("build the compositor with cargo build --locked first")
    if args.gpu:
        node = Path(args.gpu).resolve()
        if not node.is_char_device() or not re.fullmatch(r"renderD\d+", node.name):
            parser.error("--gpu requires a DRM render node")
        args.gpu = str(node)
        if Path("/proc/1/comm").read_text().strip() != "bwrap":
            wrapper = Path(__file__).with_name("render-node.sh")
            flags = ["--frame-rate", str(args.frame_rate)] if args.frame_rate else []
            os.execv(wrapper, [str(wrapper), args.gpu, "--binary", str(args.binary), *flags])
        assert all(os.statvfs(card).f_flag & os.ST_NODEV for card in node.parent.glob("card*")), "primary device must be inert"
    artifacts = ROOT / "target/nested-tests"
    artifacts.mkdir(parents=True, exist_ok=True)
    base = Path(tempfile.mkdtemp(dir=artifacts))
    print(f"Nested test artifacts: {base}", flush=True)
    # Allowlist the tool/library paths. Never inherit a display, IPC socket, bus or session.
    env = {k: os.environ[k] for k in ("PATH", "LD_LIBRARY_PATH", "XKB_CONFIG_ROOT", "GI_TYPELIB_PATH",
                                      "XDG_DATA_DIRS") if k in os.environ}
    for key in ("HOME", "XDG_RUNTIME_DIR", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME",
                "XDG_STATE_HOME", "TMPDIR"):
        directory = base / key.lower()
        directory.mkdir(mode=0o700)
        env[key] = str(directory)
    env.update(LC_ALL="C", RUST_LOG="info,smithay::backend::egl::surface=debug,smithay::backend::winit=trace,mundane_de::backend::winit=trace")
    env.update(DBUS_SYSTEM_BUS_ADDRESS="unix:path=/nonexistent", GSETTINGS_BACKEND="memory",
               GIO_USE_VFS="local")
    if args.gpu:
        env["__EGL_VENDOR_LIBRARY_FILENAMES"] = "/usr/share/glvnd/egl_vendor.d/10_nvidia.json"
    if not args.gpu:
        # On GLVND machines with both drivers installed, select Mesa directly
        # so probing a different vendor cannot pollute the software EGL trace.
        for share in env.get("XDG_DATA_DIRS", "/usr/local/share:/usr/share").split(":"):
            vendor = Path(share) / "glvnd/egl_vendor.d/50_mesa.json"
            if vendor.is_file():
                env["__EGL_VENDOR_LIBRARY_FILENAMES"] = str(vendor)
                break
        env.update(LIBGL_ALWAYS_SOFTWARE="1", GALLIUM_DRIVER="llvmpipe")
    env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=" + str(Path(env["XDG_RUNTIME_DIR"]) / "bus")
    scenario(base, args.binary.resolve(), args.gpu, env, args.everyday, args.frame_rate)


if __name__ == "__main__":
    main()
