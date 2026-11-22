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
renderer identifies as NVIDIA. Both runs use only virtual outputs. The NVIDIA wrapper needs
bubblewrap. It exposes the selected DRM render node and NVIDIA rendering
devices inside a private namespace. Primary DRM device entries are visible for
driver discovery on a nodev mount. No host display or seat socket is mounted.
The GPU run compares a GTK EGL DMA-BUF against the shared-memory reference.
The render node advertises linux-dmabuf v5 feedback and the llvmpipe run
advertises v3 formats. The presentation probe checks two increasing timestamps
and a discarded commit on each backend.

```sh
cargo build --locked --bins --examples
tests/nested/render-node.sh /dev/dri/renderD128
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

## Everyday applications

Build the compositor and the two test clients on stable Rust. Put the two GPUI
suite binaries, `mundane-terminal` and `mundane-files`, on PATH. The wrapper
mounts those binaries read-only inside the test namespace.

```sh
cargo build --locked --bins --examples
tests/nested/everyday.sh
```

Alongside the baseline tools, install bubblewrap with unprivileged namespaces,
xkbcli, Xwayland, xterm, xclip, xdotool, foot, Nautilus, GNOME Text Editor,
Firefox, the Mesa lavapipe Vulkan driver, IBus with its GTK 3 Wayland panel and
the Python IBus introspection bindings. The tested panel path is
`/usr/lib/ibus/ibus-ui-gtk3`. The harness starts its own IBus daemon and engine.
All application state, including the Firefox profile, lives in the test directory.

The wrapper creates private PID, network and IPC namespaces, an empty device
directory with standard pseudo-devices, and a private X11 socket directory.
This confines both filesystem and abstract X11 sockets. All input goes to the
headless host through its private socket. Mesa renders in software throughout.

| Clients | What the test measures |
| --- | --- |
| xdg-shell probe and GTK 4 | All four wm_capabilities and client-side decoration mode arrive before the initial configure. Pointer clicks on GTK’s own buttons maximize, restore and minimize the window. Screenshots check the result and Super+Tab restores it. The titlebar opens the compositor menu, enters fullscreen and supports moving and edge resizing. |
| GTK popup tree and Nautilus | Open a menu at the output edge, open its grabbed submenu and invoke a transient dialog. Dismiss Nautilus’s context menu with Escape, rename the selected file and verify it on disk. |
| Launch probe and GTK | A token issued for real keyboard input focuses a previously inactive GTK window. An expired token leaves the issuer focused. |
| Text Editor and Firefox | Preedit, the candidate list and the committed string travel through IBus. The candidate list appears below two different caret positions. Editor saves the expected UTF-8 bytes and the Firefox form reports its value through its page title. Switch focus mid-preedit and check the new field starts clean. |
| GTK, foot, xterm and Xwayland | Paste exact Unicode payloads in both directions, including in real terminal PTYs. Close X clients and restart the private X server. Exercise primary selection, middle click, cross-client drag and drop, and loss of a selection owner. |
| mundane-apps | Terminal and Files open on the inner socket and submit Wayland buffers using lavapipe. |

The run retains protocol traces, the ordinary-client registry and named PPM
frames under `target/nested-tests/`. The application run used GTK 4.24.1,
Text Editor 51.0, Nautilus 51.1, Firefox 158.0.1, IBus 1.5.34, foot 1.28
and Xwayland 24.1.14. The baseline NVIDIA run uses its separate render-node
invocation above.

## Frame and idle budgets

Build with stable Rust in release mode. Run the two refresh rates separately.

```sh
cargo build --locked --release --bins --examples
tests/nested/render-node.sh /dev/dri/renderD128 --binary target/release/mundane-de --frame-rate 60
tests/nested/render-node.sh /dev/dri/renderD128 --binary target/release/mundane-de --frame-rate 120
```

Each run starts a private headless Sway GLES host at 1920×1080, warms up for
three seconds and drives ten GTK windows for one minute. The fixture staggers
window mapping to keep placement springs active, drags and releases windows
and triggers wallpaper crossfades. It then stops client updates, allows three
seconds for settling and counts redraws and submissions over ten idle seconds.

The `budget.json` in the run directory contains nearest-rank p50/p95/p99.
Thread CPU time covers spring updates and composition through EGL submission.
Elapsed submission time also includes waits. The submission interval comes
from the nested monotonic clock. Missed virtual deadlines count elapsed
refresh periods beyond the first, rounded to the nearest period. The QEMU
test separately measures kernel presentation timestamps on both virtual CRTCs.

The reference run on 2026-11-22 used an Intel Core Ultra 7 265KF, GeForce
RTX 5070 through `/dev/dri/renderD128`, NVIDIA EGL 615.78.08, Linux
7.2.15-1-omarchy and stable Rust 1.100.0. The separate software-rendered
application run used llvmpipe (LLVM 23.1.3). All figures below are milliseconds
in p50 / p95 / p99 order unless otherwise labeled.

| Measurement | 60 Hz | 120 Hz |
| --- | ---: | ---: |
| Spring update CPU | 0.003 / 0.008 / 0.013 | 0.002 / 0.007 / 0.012 |
| Composition and submission CPU | 0.320 / 0.539 / 2.136 | 0.210 / 0.450 / 2.028 |
| Total update and submission CPU | 0.324 / 0.544 / 2.140 | 0.213 / 0.454 / 2.032 |
| Total update and submission elapsed | 0.354 / 0.623 / 2.195 | 0.235 / 4.996 / 8.331 |
| Submission interval | 16.663 / 16.945 / 18.476 | 8.331 / 8.553 / 10.135 |
| Frames submitted | 3643 | 7286 |
| Measured seconds | 60.700 | 60.717 |
| Missed virtual deadlines (%) | 0.000 | 0.014 |
| Repaints / submissions during 10 s idle | 0 / 0 | 0 / 0 |
