# mundaneDE

mundaneDE is an early Wayland desktop for the x86 edition of joe Linux. Its Rust
compositor uses Smithay. Drag a window near a screen edge or another window and
release it to see a damped spring move it into place.

The compositor runs in a host window or directly on multiple DRM/KMS outputs.
It runs native Wayland applications and rootless X11 windows, negotiates
client-side decorations and supports IBus text composition. The GPUI settings panel is a separate application.
It supports mixed refresh rates, fractional scaling and hotplug. The GDM session includes a bottom taskbar with Start, a tray, desktop icons and a PAM lock screen. Its desktop portals let Firefox share a display or a single window over PipeWire. The taskbar names each active share and lets you stop it.
The [roadmap](ROADMAP.md) describes when mundaneDE can become the default joe Linux session.

## Build and run in a window

Start from the repository root in a graphical Linux session with Wayland or X11
and EGL support. Use stable Rust, a C toolchain, pkg-config and the development
libraries for Wayland, EGL/GLES, DRM, GBM, libinput, libudev, libseat and
libxkbcommon and Cairo. If you use Nix with a nixpkgs channel, `nix-shell` supplies the
native dependencies and rustup.

```sh
cargo build --locked
cargo test --locked
cargo run --locked -- --winit
```

The nested compositor opens a window and logs its Wayland socket name. With the
window focused, press `Super+Return` to open a terminal. To try a specific client
from a second terminal, substitute the socket name from the log and run, for
example, `WAYLAND_DISPLAY=wayland-2 foot` with `foot` installed.

| Action | Input in the nested window |
| --- | --- |
| Move a window and snap it on release | Super or Alt + left drag |
| Close the focused window | Super+Q |
| Reload the wallpaper | Super+W |
| Restore the last minimized window | Super+Tab |
| Quit the compositor | Super+Shift+Q |

GTK and the GPUI suite can draw their own window controls. The compositor
advertises the window menu, maximize, fullscreen and minimize capabilities.
Escape goes to the focused application.

An X11 launcher can ask the running compositor for its private display.
Set `WAYLAND_DISPLAY` to the mundaneDE socket first. With the compositor
binary on your path, run `DISPLAY=$(mundane-de --x11-display) xterm`.
The command waits for Xwayland and its window manager to be ready.

To use IBus, point `MUNDANE_IBUS_PANEL` at the upstream `ibus-ui-gtk3`
executable before starting mundaneDE. The compositor gives the panel a private
Wayland connection and passes `--enable-wayland-im`. It needs a running IBus
daemon in the same session. GTK and Firefox can then use text-input-v3.
The [isolated application tests](tests/nested/README.md#everyday-applications)
exercise this configuration.

For direct rendering on a free seat, omit `--winit`. The repository includes
an [automated two-output DRM/KMS test in QEMU](tests/qemu/README.md#automated-output-test).

## GDM session

Run `./tests/try-mundane.sh` for a persistent 1920×1080 joe Linux test guest. The launcher uses the frozen beta1 image as read-only backing.
[Guest setup](tests/qemu/session/README.md#try-the-desktop)

The optional GDM entry runs the compositor on the logged-in user's logind seat.
Super+Shift+Q returns to GDM. The session and its packaging dependencies are
qualified on the beta1 x86 joe Linux image in a headless KVM guest.
[Packaging handoff](docs/plan.md#4-enter-and-leave-a-real-session) · [GDM test](tests/qemu/session/README.md)

## Configuration

The compositor reads `$XDG_CONFIG_HOME/mundane/config.toml`, falling back to
`~/.config/mundane/config.toml` when `XDG_CONFIG_HOME` is unset. The built-in
Afterlight wallpaper works without configuration. To use another PNG or JPEG,
set `path` in the `[wallpaper]` section. Spring parameters, snap distances and
wallpaper settings can be changed while the compositor runs. See the
[example configuration](config.example.toml).

Set `reduced_motion = true` to keep the same magnetic snap targets and switch
wallpapers immediately. In a DRM session, use `wayland-info` to find connector
names and advertised modes. For example, if DP-1 advertises 1920×1080 at
60.000 Hz, these commands try it at 150% scale to the left of the primary
display, then save it after the first page flip.

```sh
mundane-de output test DP-1 1920 1080 60000 1.5 0 --position -1280 0
mundane-de output confirm
```

Use `mundane-de output revert` or wait 15 seconds to recover the previous
layout. Confirmed preferences are stored in `$XDG_STATE_HOME/mundane/displays.toml`
(default `~/.local/state/mundane/displays.toml`) and restored on reconnect.

The optional settings panel builds from its own Cargo workspace. Once its binary
is on your path, `Super+,` opens it. You can also point `MUNDANE_SETTINGS` at
the binary before starting the compositor. See the [settings panel guide](settings-panel/README.md).

## Licence

The compositor and its original Afterlight wallpaper use the
[BSD 3-Clause licence](LICENSE). The settings panel retains its MIT licence.
Earlier Git history contains `pexels-jplenio-2080960.jpg`, a photograph by
Johannes Plenio distributed under the Pexels Licence. For credits and dependency
licences, see [third-party notices](THIRD-PARTY-NOTICES.md).
