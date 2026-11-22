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
