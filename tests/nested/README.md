# Isolated nested compositor test

From the repository root, build with stable Rust and start the test as an
ordinary user. It needs Python 3 with PyGObject and GTK 4.12 or later, Sway
and swaymsg, grim, wayland-info, dbus-daemon and Mesa's llvmpipe EGL driver.

```sh
cargo build --locked
python3 tests/nested/run.py
```

The harness creates a headless Sway output with the pixman renderer and runs
mundaneDE's winit backend inside it with llvmpipe. Each invocation starts with
its own home, XDG directories, Wayland sockets, Sway IPC and D-Bus daemon.
The bus has no activatable services. The harness addresses its own sockets
explicitly. It needs no desktop login, display server or VT on the host.

To run the same regression against the NVIDIA driver, supply its accessible
DRM render node. This uses a headless GLES host and verifies the nested GL
renderer identifies as NVIDIA. Both runs use only virtual outputs.

```sh
python3 tests/nested/run.py --gpu /dev/dri/renderD128
```

Each cold start checks the ordinary-client registry against the checked-in
[baseline](registry.txt). The test samples Afterlight on a 640×360 output and
again after resizing the host output to 800×450. Five RGB samples allow a
six-level per-channel difference for filtering. A real undecorated GTK client
acknowledges its xdg configure, maps a green 120×80 shared-memory buffer and
requests a resize to 200×110. The harness checks both buffer sizes in the
protocol trace and the exact green rectangle on the host output. Closing the
client must restore the settled wallpaper byte for byte. The compositor must
exit normally, issue buffer-age queries after initialization and log no EGL
errors.

The run prints its artifact directory under `target/nested-tests/`. Each
named PPM contains the last observed frame for that check. The directory also
contains the full registry, GTK Wayland trace, and compositor and host logs.
Pass `--binary PATH` to check another build with the same fixture.
