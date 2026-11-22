# Replacing GNOME 51 on x86 joe Linux

The first distribution deliverable is a second session in GDM. A default
change requires automated qualification on fresh and upgraded installations
followed by the maintainer’s sign-off after a real-machine test. Application replacement has a separate schedule. A working
desktop must be able to run today's Nautilus, Text Editor and Firefox while the
GPUI suite matures. [Public roadmap](../ROADMAP.md) · [joe desktop package](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/packages/joe-desktop/package.zon)

## Starting point and responsibilities

At `8fc7b43`, both backends share a Smithay `Space`, one output, a GLES
renderer, spring placement and a wallpaper engine. The compositor registers
xdg-shell, shared memory, seat, output and data-device globals. A toplevel
receives an initial configure and maps on its first buffer. Popup creation,
grabs and repositioning are empty handlers. Output geometry assumes scale one.
DRM selects one connector, and session pause currently suspends libinput. Both
loops wake on a 16 ms timeout. The GPUI panel writes compositor TOML and talks
to system services. Its Display page talks to the DRM session’s authenticated output socket. [Shell](../src/shell/mod.rs) ·
[State](../src/state.rs) · [DRM backend](../src/backend/drm.rs) · [Settings panel](../settings-panel/README.md)

The target is x86/glibc with systemd 262 and logind, Mesa 26.2, GTK 4.24,
libadwaita 1.10, Xwayland 24.1, xdg-desktop-portal 1.22 and PipeWire 1.6
with WirePlumber. GNOME Shell currently supplies the panel and launcher via
Dash to Panel and ArcMenu. Keep GDM, the first-account flow, keyring, GVfs,
fonts, icons and MIME handlers available during the transition. The distribution
maintainer owns package recipes, neomake VM integration and the eventual
default. This repository supplies the compositor, shell, settings panel,
session files and reproducible test scenarios. [Distribution package list](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/packages/joe-desktop/package.zon)

## 1. Establish a repeatable compositor test

Run the locked compositor and panel workspaces on stable Rust. Keep Smithay and
smithay-drm-extras together at v0.7.0 (`a166cf4`) for the baseline. At a
milestone that needs an upstream fix or protocol, identify the missing API,
prefer a released tag and pin an exact commit if an unreleased fix is necessary.
Update both dependencies, compatible calloop/xkbcommon versions, the lockfile
and the pin record together. Review protocol version changes, redo the registry
and screenshot checks, and exercise DRM pause/resume before accepting the
upgrade. Keep the previous lockfile recoverable with a normal revert. [Smithay pin](../PINNING.md)

**Nested EGL finding.** The initial `eglQuerySurface` for buffer age used to run
before the winit surface was bound, raising `EGL_BAD_SURFACE` on NVIDIA.
The compositor now uses age zero until a successful submission. It queries
again only when the window size still matches and the EGL surface is current.
A rendering error invalidates the recorded size. The private headless
regression reproduced the old failure and passed with the fix on both Mesa
llvmpipe and an NVIDIA RTX 5070. [Nested render path](../src/backend/winit.rs) ·
[Smithay buffer-age contract](https://github.com/Smithay/smithay/blob/a166cf4c94b5aedc332a65aa1dd753e8148829c3/src/backend/winit/mod.rs#L305-L320)

**Acceptance.** From private XDG directories and a private session bus, start
winit inside a headless Wayland host with Mesa software rendering. Wait for
socket readiness, map a real xdg-shell fixture with a known colour, resize it,
assert the configure/ack and new buffer dimensions, then close it. Capture the
host output before mapping, after resize and after close. Require the expected
pixels and a clean child exit. Repeat cold starts and resize on the affected
driver with buffer-age tracing. Check for zero startup EGL errors and correct
damage on later frames. The [nested test](../tests/nested/README.md) records and checks the ordinary-client
registry. Use this run for the
first Terminal integration with the app suite, including its keyboard-focus
and screen-reader probe (milestone 7).

## 2. Make everyday clients work

Treat protocol support as behaviour a client can exercise. Record the global
name and negotiated version alongside the client, backend and test result.
Use the upstream XML and the version shipped with the client when implementing
each row. [Wayland protocol definitions](https://gitlab.freedesktop.org/wayland/wayland-protocols) ·
[wlr protocol definitions](https://gitlab.freedesktop.org/wlroots/wlr-protocols)

| Interface | Behaviour and protocol test |
| --- | --- |
| xdg-shell and xdg-decoration | Handle popup trees, serial-checked grabs, reposition/dismiss, interactive move/resize, maximized and fullscreen configure/ack. Negotiate client-side decorations for GTK and a usable decoration mode for other clients. Exercise an edge-positioned menu, a nested submenu and a transient dialog in Nautilus. |
| wl_data_device, primary-selection and data-control | Update selection on focus changes. Test clipboard, drag-and-drop and middle-click selection across two clients, including source exit. Offer ext-data-control only to an authorized clipboard manager, with wlr-data-control compatibility if the selected manager needs it. |
| text-input and input-method | Connect text-input-v3 clients to a trusted input-method-v2 frontend backed by IBus. Forward preedit, commit, surrounding-text changes, focus and caret geometry. On joe, provide a session frontend talking to the packaged IBus daemon and a candidate popup. The joe recipe disables the upstream Wayland frontend and GTK input modules. Test with a fixture engine that produces candidates and complex text. [joe IBus build](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/packages/ibus/build.sh) |
| cursor-shape and xdg-activation | Honour cursor changes over text, links and resize edges. Use recent seat serials and activation tokens for launches and notification actions. A valid user launch focuses its window, while a stale request leaves focus where the user put it. |
| Xwayland and xwayland-shell | Start rootless Xwayland on demand, associate X11 windows with Wayland surfaces through Smithay's XWM, and bridge focus, transient windows, clipboard ownership and close requests. Exercise native and X11 clients side by side and restart Xwayland after closing the X clients. |

**Acceptance.** Open a folder and rename a file in Nautilus, dismiss its menus
with Escape, and type immediately in the same window. Scope the prototype's
global Escape-to-quit shortcut to development use. Compose and commit the same
string in GTK Text Editor and a Firefox form, switch focus mid-preedit and check
the candidate popup follows the caret. Copy in both directions between
`xterm` and a native terminal. Run the suite's Terminal and Files as additional
native clients when their first runnable revisions arrive.

**Nested result.** The application harness passes with real GTK title buttons,
the compositor window menu, interactive move and resize, a nested popup tree,
Nautilus rename and fresh and expired activation tokens. It checks both terminal
clipboards after restarting Xwayland, middle-click paste, drag and drop, and
selection-owner exit. A fixture IBus engine shows Japanese candidates at two
caret positions in Text Editor. Firefox receives the same committed text in a
form and starts a fresh composition when focus changes. Terminal and Files from
mundane-apps each submit a buffer. The ordinary registry, negotiated versions,
traces and screenshots are recorded by the [application tests](../tests/nested/README.md#everyday-applications).

**Distribution handoff.** Package Xwayland on PATH. The launcher can obtain
`DISPLAY` from `mundane-de --x11-display`. Ship the upstream GTK 3 IBus panel
built with its Wayland input-method-v2 frontend, start the session's IBus daemon
with the automatic panel disabled, and set `MUNDANE_IBUS_PANEL` to the panel
executable. The compositor launches it with `--enable-wayland-im`. This panel
draws the candidate list and auxiliary text in an input-method popup. mundaneDE
positions that surface by the focused field’s caret, keeping it on the output.
The panel relays preedit and commits through input-method-v2 and text-input-v3.
GTK and Firefox draw inline preedit in the field and insert the committed text.
Keep the existing joe IBus daemon and engines. The compositor gives
input-method and virtual-keyboard access to that frontend’s private connection.
An optional session clipboard manager named by `MUNDANE_CLIPBOARD_MANAGER`
receives ext-data-control on its own connection. GTK and GPUI should use
client-side controls according to the negotiated mode and xdg-toplevel
capabilities. The taskbar restore path belongs to milestone 5.

## 3. Drive each display at its own scale and refresh rate

Each active CRTC has a scanout, damage history, pending flip and presentation
clock. The session maps connectors into logical coordinates and picks the output
with the largest overlap for each surface. Unplugging a display keeps windows,
pointer and keyboard focus reachable. A mode trial restores the previous layout
after 15 seconds unless confirmed after a successful flip. Confirmed connector
preferences survive reconnect and login. The shell in milestone 5 will use
this layout for its panels.

| Interface | Behaviour and test |
| --- | --- |
| fractional-scale-v1 and viewporter | Send preferred fractional scale, apply viewport source/destination correctly and retain integer-scale fallback. Move GTK, Firefox and GPUI between 100%, 150% and 200%. Check a sharp text sample, exact pointer hit targets and a popup crossing the boundary. |
| linux-dmabuf and presentation-time | Advertise formats/modifiers the renderer imports, send feedback and retain the shared-memory path. Exercise a Mesa/EGL client and a GPUI wgpu client, compare colours, and check presented/discarded feedback against DRM page flips. Record the timing source on nested runs. |
| tearing-control-v1 | Treat the requested presentation mode as a hint. Use synchronized flips for all windows, including fullscreen surfaces requesting async. The pinned Smithay DRM compositor exposes synchronized submission. The guest requests async and checks vsync feedback. |

**Acceptance.** With two outputs of different scale and refresh, drag a window
between them, unplug its current output, reconnect, then reject a proposed mode
and recover the previous layout. Check window bounds, cursor, focus and content
in each state. Exercise hotplug, modesetting and kernel page-flip feedback on virtual connectors
in a headless DRM guest. Check DMA-BUF import against shared-memory pixels on
the isolated render node. Physical displays belong to the final maintainer gate.

Magnetic placement should go unnoticed during an ordinary drag. Verify the
window follows the pointer until release, settles at a nearby edge or neighbour
and stays put when released outside the attraction radius. Repeat with reduced
motion enabled and use the same target immediately, without overshoot or a fade.

**Spring and frame budgets.** Measure the release build on a recorded GPU/CPU
with headless 1920×1080 outputs at 60 and 120 Hz. Script ten moving windows and repeat drag, snap
and wallpaper fade for 60 seconds after warm-up. Record p50/p95/p99 spring
CPU time, composition time and presentation intervals per output. Target p95
spring update below 1 ms, p95 CPU update plus render submission below 8 ms at
60 Hz and 4 ms at 120 Hz, and fewer than 1% missed refresh deadlines in each
run. Drive frames from damage, active springs and pending client work. In a
10-second settled scene with a stationary cursor and no client updates, expect
zero animation redraws. In a deterministic clock test, drop a frame, check
bounded motion and exact final positions, then verify the final settled frame
reaches the screen. Keep software-rendered screenshot runs separate from timing
measurements. The nested timing source is the monotonic EGL submission timestamp.
DRM clients receive the guest kernel’s monotonic flip timestamp and sequence
when available. Superseded submissions receive discarded feedback.

The [two-output guest test](../tests/qemu/README.md#automated-output-test) covers
mode rollback, reconnect, application scaling and per-output presentation. The
[frame and idle measurements](../tests/nested/README.md#frame-and-idle-budgets)
record the reference run and reproduction commands.

## 4. Enter and leave a real session

GDM starts `mundane-session` in the authenticated user's logind seat scope.
The launcher starts the compositor there and selects libseat’s logind backend.
After the Wayland socket, DRM outputs and input sources are ready, a private
socket connects the compositor and launcher. The launcher publishes the
display and desktop to D-Bus activation and the systemd user manager, starts
the session target, waits for the shell’s portal backend, then restarts the
portal frontend. The compositor starts the panel in the same seat scope. The
panel registers its polkit agent there. Settings and applications run as the
logged-in user. [systemd session targets](https://www.freedesktop.org/software/systemd/man/latest/systemd.special.html) · [joe security model](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/README.md#L174-L215)

### joe packaging handoff

The [GPUI shell migration](shell-gpui.md) has a separate preview entry. To package the preview for testing, pass the release `mundane-shell` binary as the second argument to `session/install.sh`. Build it from the archive with `cargo build --release --locked --offline --manifest-path settings-panel/Cargo.toml -p mundane-shell`. It installs `$prefix/bin/mundane-shell` and `/usr/share/wayland-sessions/mundane-gpui.desktop`. The existing mundaneDE entry selects the Python reference. The preview uses GIO, the desktop and favourite schemas, dconf, the icon theme, fonts and the same Mesa Vulkan loader/ICD as Settings. Quick Settings uses the device services and clients listed below, including the Pulse-compatible PipeWire service, and publishes the appearance Settings backend on the session bus. Native Settings dialogs use the shared GPUI text and password fields. Both Rust GUIs share the Settings workspace lockfile. [Build the preview](../tests/qemu/session/README.md#gpui-shell-step-1) · [Qualify Quick Settings](../tests/qemu/session/README.md#gpui-shell-step-2)

In the step 3 preview, the same shell binary owns notifications, the ScreenCast and Screenshot backends and their GPUI consent UI. For an approved stream it starts `dbus-run-session -- mundane-shell --cast-worker …` with a source-limited Wayland fd and invokes xdg-desktop-portal-wlr on that private bus. For a still it invokes grim. Install the portal implementation XML, routing files, PipeWire/WirePlumber and helpers in the table below. The preview reads the same published dave status and notice state as the reference. [Preview portal and notification tests](../tests/qemu/session/README.md#gpui-shell-step-3)


**Vendored source archive.** Run `tools/dist.sh` in a Git checkout. It exports
the committed HEAD, reads the compositor version and writes
`target/dist/mundaneDE-VERSION.tar.xz` and prints its SHA-256. One `vendor/`
and one archive-relative `.cargo/config.toml` serve both original lockfiles,
including Git dependencies. Making the archive needs Bash, Git, Python 3.11+
with tomllib, Cargo, GNU tar, coreutils and xz. Cargo can fetch on the packaging
host or use a populated cache with `CARGO_NET_OFFLINE=true`. Rebuild with the
same commit, tool versions and epoch to reproduce the bytes. The default epoch
is the commit time, with an override through `SOURCE_DATE_EPOCH`.
[Desktop archive script](../tools/dist.sh) · [Suite archive convention](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/7854078/docs/plan.md#vendored-source-archive)

From the extracted archive root, with an absolute staging directory in
`pkgdir`, build both workspaces against their own lockfile and stage the payload:

```sh
cargo build --release --locked --offline
(cd settings-panel && cargo build --release --locked --offline)
PREFIX=/usr DESTDIR="$pkgdir" session/install.sh target/release/mundane-de
install -Dm755 settings-panel/target/release/mundane-settings "$pkgdir/usr/bin/mundane-settings"
for f in LICENSE THIRD-PARTY-NOTICES.md licenses/Smithay-MIT.txt settings-panel/LICENSE; do
    install -Dm644 "$f" "$pkgdir/usr/share/licenses/mundaneDE/$f"
done
```

Supply stable Rust/Cargo, a C/C++ build toolchain, pkg-config and clang/libclang
in the joe build root. Provide native development files for the runtime
libraries below, including Wayland protocols/scanner, EGL/GLES/GBM, libdrm,
libinput, libudev, libseat, Cairo, xkbcommon, Fontconfig/FreeType, X11/XCB,
ALSA, D-Bus and OpenSSL. The archive supplies the Rust sources. The native
development environments are listed in the [compositor shell](../shell.nix)
and [Settings shell](../settings-panel/shell.nix).

| Package-owned directory | Files to install |
| --- | --- |
| `/usr/bin/` | `mundane-de`, `mundane-settings`, `mundane-session`, `mundane-panel`. The first two are native executables. |
| `/usr/share/wayland-sessions/` | `mundane.desktop`, generated with both `Exec` and `TryExec` pointing to `/usr/bin/mundane-session`. |
| `/usr/lib/systemd/user/` | `mundane-session.target`, `mundane-ibus.service`. The launcher starts the target after publishing the session environment. |
| `/usr/share/mundane/shell/` | `appearance.py`, `applications.py`, `authentication.py`, `bluetooth.py`, `bus.py`, `cast.py`, `devices.py`, `panel.py`, `portals.py`, `quick_settings.py`, `reader.py`, `tray_menu.py`, `upgrades.py`. |
| `/usr/share/glib-2.0/schemas/` | `50-mundane.gschema.override`, `org.joelinux.mundane.shell.gschema.xml` (taskbar favourites). Run joe’s `glib-compile-schemas` trigger before sealing the image. |
| `/usr/share/xdg-desktop-portal/` | `mundane-portals.conf`, `portals/mundane.portal`. |
| `/usr/share/licenses/mundaneDE/` | `LICENSE`, `THIRD-PARTY-NOTICES.md`, `licenses/Smithay-MIT.txt`, `settings-panel/LICENSE`. Upstream crate licences accompany their sources in the archive. |

The compositor embeds the Afterlight wallpaper. User configuration is created
under XDG paths. [Session installer](../session/install.sh)

| Runtime on the joe desktop image | What uses it |
| --- | --- |
| GDM, AccountsService, PAM (`gdm-password`, `gdm-autologin`, pam_systemd), systemd-logind and user manager, system and user D-Bus, GLib | Keep GNOME selectable. Opt in per user with AccountsService `Session=mundane` and `SessionType=wayland`. Supply `systemctl`, `busctl`, `dbus-update-activation-environment` and `gdbus`. The launcher publishes `WAYLAND_DISPLAY`, desktop/session type and session ID. Logout stops both session targets and clears activation state. |
| Wayland, libseat with logind support (joe’s seatd package), libinput, udev, libdrm, Mesa GBM/EGL/GLES and the device driver, Cairo, libxkbcommon, xkeyboard-config, Xcursor theme, Xwayland | Compositor, VT switch and DRM resume. The launcher holds logind’s sleep delay inhibitor until scanout stops. Use the display from `mundane-de --x11-display` for X11 clients. GPUI Settings also needs a Vulkan loader and working ICD, Fontconfig/FreeType and the X11/XCB/xkbcommon-x11 libraries for its compiled backends. |
| Python 3, PyGObject, GLib/Gio and GTK4/Gdk typelibs, gtk4-layer-shell (both `libgtk4-layer-shell.so.0` and `Gtk4LayerShell-1.0` typelib), gsettings-desktop-schemas, dconf, fonts and desktop icon theme | The panel, launcher, Quick Settings and live themes. Include Fontconfig’s `fc-list`, XDG user directories, MIME data, desktop entries and registered browser/file handlers. Provide a terminal (set `MUNDANE_TERMINAL=mundane-terminal` to select the suite’s Terminal). |
| polkit (`polkitd`, Polkit and PolkitAgent 1.0 typelibs, `polkit-agent-helper-1`, `polkit-1` PAM service, `pkexec`), joe administrator policy, swaylock with its PAM service | The panel is the graphical authentication agent for its logind session. The GPUI preview links the system’s `libpolkit-agent-1` and `libpolkit-gobject-1`. Include their runtime libraries and retain the typelibs for the GTK reference session. Both sessions provide their own agent. Lock uses swaylock and joe’s system authentication stack. |
| xdg-desktop-portal with implementation XML, xdg-desktop-portal-gtk, gnome-keyring with its Secret backend, xdg-desktop-portal-wlr 0.9.0, grim 1.5.0, `dbus-run-session`, PipeWire and WirePlumber | The shell handles capture consent, revocation and appearance. The wlr helper runs on a private bus. GTK provides chooser and notification interfaces, and keyring handles secrets. See milestone 6 for helper lookup and routing. |
| IBus with its GTK3 Wayland panel and an installed conversion engine | Supply `ibus-daemon` and `ibus-ui-gtk3 --enable-wayland-im` in `/usr/libexec` or `/usr/lib/ibus` (override the executable with `MUNDANE_IBUS_PANEL`). The session unit runs the daemon with `--panel=disable`. The compositor connects its own frontend for preedit and candidate selection in GTK and Firefox. |
| AT-SPI2 2.62.1 or compatible, Orca (qualified with 51.1), Speech Dispatcher and a voice such as eSpeak NG | The shell starts `/usr/bin/orca` and grants its session-bus connection the keyboard monitor needed for typing echo. |
| NetworkManager and `nmcli`, BlueZ and `bluetoothctl`, UPower, WirePlumber’s `wpctl`, PipeWire Pulse service and `pactl` client | Network, Bluetooth, battery and audio controls in the shell and Settings. Brightness uses sysfs and logind on machines with a backlight. |
| timedated / `timedatectl`, shadow’s `chpasswd`, Flatpak for the remotes page, the suite’s `mundane-packages` | Settings’ time, accounts, software and System pages. Protected actions use joe’s polkit/PAM policy. |
| dave with `dave status --json`, `dave.status/1` and `prepared_upgrade.id`, plus `dave-status.service` | Root publishes at boot and after deployment changes. The ordinary user’s panel reads the snapshot and remembers the delivered ID or fallback event. See milestone 5 for expected notices. |

Ctrl+Alt+Fn asks libseat to change VTs. On VT loss, suspend libinput and DRM.
On return, reacquire devices and refresh outputs. Before system sleep, stop scanout
and release its buffers. On wake, rebuild the scanout state and repaint. Logout
releases the compositor’s sockets and clears the manager’s display variables.
Classic D-Bus replaces stale activation values with empty strings.

**Acceptance.** In a headless KVM guest booted from the read-only joe image,
qualify fresh GDM logins, D-Bus activation, VT loss and return, S3 and wake,
protected Settings operations, logout cleanup and a subsequent GNOME login.
Use changed scanout pixels and delivered input to verify recovery. The final
real-machine sign-off remains in milestone 8. [Session qualification](../tests/qemu/session/README.md)

Two beta1 observations carry forward to image qualification. Under enforcement,
joe logged two `CAP_SETUID` denials for `polkit-agent-helper-1` during a
successful Settings authentication. Review the helper’s expected privileges
against joe’s package policy. The guest kernel also logged virtio-gpu response
`0x1203` across S3 in both mundaneDE and a GNOME control run. The mundaneDE
client repainted and received keyboard and pointer input after wake.

## 5. Provide a shell, workspaces and a lock screen

Preserve the bottom taskbar, Start menu, StatusNotifierItem tray and desktop
icons. Show the user’s XDG Desktop directory, support keyboard selection and
open items through their preferred handler. Provide minimize and maximize
buttons on normal resizable windows. Agree decoration mode with each toolkit,
configure GTK header bars and implement the controls on server-decorated
windows. Test both kinds. Minimize to the taskbar, restore from it, maximize
up to the taskbar and restore the original bounds. Launch from Start and from
a desktop icon, and activate a tray item. Compare the default layout against
the joe GNOME session at the same resolution. [Desktop announcement](https://joelinux.io/news/mundanede/)

Deliver the panel, launcher, notification service, on-screen displays (OSDs)
and system menu as restartable session components. Read installed desktop
entries, match Wayland app IDs, honour MIME/URI handlers and use activation
tokens. Provide a task list, workspace switching, a keyboard-searchable
overview, notification actions/history and a visible sharing indicator with a
stop control. Support StatusNotifierItem tray applications used on joe.
The compositor owns placement and workspace membership across shell restarts.

| Interface | Consumer and acceptance test |
| --- | --- |
| wlr-layer-shell | The panel reserves its exclusive zone on each output. Test a maximized window beneath it, a keyboard-interactive launcher, and a transient volume OSD that leaves typing focus with the app. |
| foreign-toplevel | Use ext-foreign-toplevel-list for enumeration. Give the trusted shell a control path for activate, close and workspace moves. Listing alone cannot perform those actions. Match two windows of one app, move one and recover the correct focus after closing it. |
| idle-inhibit, idle-notify and ext-session-lock | Play a video with a visible inhibitor, hide it and verify idle policy resumes. Explicit lock always wins. Cover every output before reporting locked, route input only to the locker, and cover newly connected outputs immediately. Keep a solid locked surface if the locker disappears. Authenticate unlock through PAM. |
| security-context-v1 | Track the sandbox context associated with new connections. Filter capture, input-method, data-control, lock and shell-management globals by the authorized connection. Establish trusted session components through compositor-issued connections. Treat self-reported app IDs as display metadata. Compare ordinary, sandboxed and authorized registries and operations. |

Connect the system menu to NetworkManager for network state and connections,
PipeWire/WirePlumber for devices, default sink and volume, UPower for battery
status, logind for suspend/power actions, and BlueZ for pairing and connection.
Mirror device changes made elsewhere. Use a polkit agent when the service asks
for authorization. Handle a missing radio, disappearing sink and cancelled
pairing as visible, recoverable states.

**Acceptance.** With two workspaces, launch and find an app by keyboard, move
its window, activate a notification and recover the previous focus on dismiss.
Toggle Wi-Fi, change audio output, show its volume OSD, pair a test device and
read battery status. Restart the panel and retain windows/workspaces. Lock two
displays, attach a third, terminate the locker, and verify coverage and
input isolation until a recovered locker authenticates. Include a suspend/lid
cycle with the session locked before sleep. Exercise service failures on a
private test bus and repeat device actions on a laptop.

**Implementation and joe packaging.** Install the panel launcher and its Python
modules with `session/install.sh`. Supply GTK 4, PyGObject, the
gtk4-layer-shell shared library and typelib, icon themes, GIO desktop and MIME
databases and the distribution’s preferred file handlers. The session launches
the panel on a compositor-issued Wayland connection. It uses a second,
ordinary connection for ext-foreign-toplevel enumeration. Window activation,
close and workspace moves travel on a private socket inherited by the panel.
A replacement panel enumerates the existing windows again.

Ship swaylock with its PAM service (`auth include system-auth` on the test
image), NetworkManager and nmcli, PipeWire, WirePlumber and wpctl, BlueZ
and bluetoothctl, UPower, polkit and logind. The compositor launches the PAM
locker on its own connection. It masks every output, checks page-flip completion
and keeps drawing that mask if the locker exits. The session handles logind’s
Lock and PrepareForSleep signals. Explicit lock overrides an idle inhibitor.

Milestone 6’s portal backend owns `org.freedesktop.impl.portal.desktop.mundane`.
Export its active sessions through ObjectManager at
`/org/freedesktop/portal/desktop`. Each stream session publishes
`org.joelinux.mundane.ActiveShare.Source`, a user-facing display name.
Emit InterfacesAdded/Removed as sharing starts or ends and implement
`org.freedesktop.impl.portal.Session.Close` on the same path. The shell
shows a stop button for each active source. The portal qualification exercises this control against live Firefox streams.

At 1920×1080, beta1 GNOME and mundaneDE both place Home and Trash at the upper
left, Start at the lower left and system controls at the lower right. GNOME also
pins application launchers. mundaneDE uses the space beside Start for running
windows. The headless shell checks drive the guest’s actual taskbar, inspect
scanout and exercise PAM. Virtual radios and sinks use NetworkManager and
WirePlumber. BlueZ and UPower service tests run on a private bus. Physical
device and lid checks belong to the maintainer’s real-machine gate.
[Shell tests and hand-test launcher](../tests/qemu/session/README.md#try-the-desktop)

### Upgrade and recovery status on the desktop

The shell owns a persistent entry in the system menu and a notification when
an upgrade is prepared or a failed upgrade has returned the machine to its
previous system. Show these at login and refresh during the session, including
when Packages is closed. Offer a normal, user-confirmed restart for a prepared
upgrade. Keep a recovery notice available after its notification is dismissed.

Run `dave status --json` as the session user at login and after resume, then
poll its opaque `revision` for changes. In dave 0.4.0-r3 on the `dave-json`
branch, `dave-status.service` publishes the snapshot at boot and privileged
dave operations refresh it after changes. Read `dave.status/1` from stdout.
Ignore added fields. For an error, unsupported schema or incomplete snapshot,
mark status unavailable and retry without issuing or acknowledging a notice.
An unmanaged system has no deployment notices. [dave(8), JSON](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d5c1bfab75d61a3d8492ddd09e0b7cc62e425979/dave/share/dave.8#L309) · [Status types and outcome rules](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d5c1bfab75d61a3d8492ddd09e0b7cc62e425979/dave/src/answers.zig#L174)

| State in a complete, managed snapshot | Shell behaviour |
| --- | --- |
| `prepared_upgrade` is present | Show “Upgrade ready, restart to apply” with its `summary`, `install` and `remove` counts. Clear the entry when the value returns to null. A read-only `dave upgrade --check --json` preview belongs to Packages and cannot raise this notice. |
| `last_upgrade.outcome == "fallback"` | Report that deployment `last_upgrade.deployment` failed and the machine returned to deployment `running`. Show the supplied `message` in the details. Only dave classifies exhausted systemd-boot attempts. |
| `not_running`, `checking`, `succeeded`, `unknown`, or null `last_upgrade` | Suppress recovery alerts, including after a manual rollback or boot-menu choice. `not_applied` can show its reason in the menu as a shutdown application failure. |

Use `running`, `next_boot` and the `deployments` records (health `good`,
`untested`, `failed` or null) for details. Comparing IDs or health cannot
establish an automatic fallback. Store the delivered fallback’s `event` per
user across shell restarts. Seeing the same event in `checking` must leave a
later fallback eligible for notification.

Remember each delivered `prepared_upgrade.id` across polls, clears and
logins. dave hashes the installed package set and the packages to install
or remove. Two preparations with equal counts can therefore produce distinct
notices. An older snapshot can be matched by its summary and counts until an
ID is available. [Prepared upgrade identity](https://github.com/theJoeLinuxFoundation/joe-linux/commit/ab99159)

**Acceptance.** With Packages closed, exercise preview, prepared, successful
boot, manual rollback and automatic fallback. Check the attempted and restored
IDs against joe’s deployment VM scenarios. Restart the shell and repeat a poll
after dismissal. The system menu retains the current state and a delivered
event stays quiet. Also exercise incomplete, unavailable and unsupported data
as an ordinary user. [Deployment VM scenarios](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d5c1bfab75d61a3d8492ddd09e0b7cc62e425979/tests/vm/deployments.tomlx)

## 6. Deliver portals that work outside GNOME Shell

The GDM session sets `XDG_CURRENT_DESKTOP=mundane`. Install
`share/xdg-desktop-portal/mundane-portals.conf` and
`share/xdg-desktop-portal/portals/mundane.portal`. The frontend routes
ScreenCast and Screenshot to the shell, Settings to both the shell and GTK,
FileChooser, AppChooser and Notification to GTK, and Secret to GNOME Keyring. OpenURI uses the
preferred desktop handler. The shell owns its implementation bus name for the
life of the session. [Portal routing](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html)

The shell names the requesting application when the frontend supplies its ID.
Its chooser lists connected displays and individual windows and initially
focuses Cancel. Selecting a source passes a fresh Wayland connection to one
helper. The compositor checks both the connection and the selected source.
Ordinary and sandboxed clients see no capture globals. A window capture
renders that window’s surface tree, including when another app covers it.
The compositor revokes grants before drawing the lock surface and on source
removal or panel loss. [Compositor capture](../src/capture/mod.rs) ·
[Desktop backend](../session/shell/portals.py)

For joe packaging, provide **xdg-desktop-portal-wlr 0.9.0**, grim 1.5.0,
`dbus-run-session`, PyGObject, the xdg-desktop-portal implementation XML and
the session’s PipeWire service and WirePlumber policy. Keep the executable
available on PATH or in the backend’s usual libexec directory. Each approved
stream gets its own wlr process and private D-Bus session. It uses
ext-image-copy-capture-v1 with an output or foreign-toplevel capture source,
negotiates shared-memory video buffers and caps capture at 30 fps. The
frontend receives the PipeWire node and its serial and gives the application
a restricted remote. The shell shows a stop control on each taskbar until the
stream ends. For a still image, grim uses wlr-screencopy v1 for a display
or ext-image-copy-capture for a window. The returned PNG lives in the user’s
runtime directory. [Pinned wlr release](https://github.com/emersion/xdg-desktop-portal-wlr/tree/8388b8a6214470462146840e92909aff5983eeac) ·
[ScreenCast API](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)

**Acceptance.** On a private bus, start a Firefox WebRTC share with two outputs
showing different test patterns. Select one, verify the received PipeWire
frames contain only that output and see the sharing indicator. Stop from the
shell, cancel a second request, then test source unplug, browser exit and session
lock during a share. Verify stream teardown and no lock-screen pixels. Request
a screenshot, check the returned URI and its pixels, then deny a fresh request
and check that no image is delivered. Repeat source isolation for a single
window before advertising window sharing. From Files or a suite fixture, open
and cancel a file chooser, open a URI in its handler, and activate a
notification. Check the active backend names in the session journal.

The headless beta1 gate passed with two QEMU virtio displays. The selected
display’s PNG matched both QMP colour samples exactly. Firefox reproduced
both bands through WebRTC within four RGB levels per channel. Firefox and
a GStreamer client using the restricted PipeWire remote both captured the
covered window. The gate checks that the node and stop control disappear
after teardown. [Repeat the portal qualification](../tests/qemu/session/README.md#headless-portal-qualification)

## 7. Share settings, appearance and accessibility

Keep the MIT-licensed GPUI Settings application in this repository.
`mundane-settings-core` owns validation, migrations and the system adapters.
Keep spring, gaps and wallpaper in the compositor's XDG TOML file. Preserve
comments and prior keys, write atomically, validate numbers before applying,
and retain working settings after an invalid edit. Give Settings an
authenticated output-management path with test/apply/revert for display
layouts. Replace the Wayland display stub and keep NixOS upgrade commands out
of the joe workflow. Direct package management to the suite's Packages app
when its dave integration is ready. Use AccountsService, timedated and the
same device services as the system menu for the relevant pages.

**Shell-owned polkit agent.** The shell registers
`org.freedesktop.PolicyKit1.AuthenticationAgent` with the system authority for
the active logind session and unregister at logout. Accept BeginAuthentication
and CancelAuthentication from the authority. Present the action’s localized
message, action ID and details in an accessible shell dialog, and let the
user choose among the administrator identities the authority offers. Use
libpolkit-agent’s authentication session and the distribution’s
`polkit-agent-helper-1`/PAM conversation. Keep passwords out of logs,
command arguments and saved state. Give the dialog keyboard focus and restore
the requesting window afterwards. Cancel pending authentication on lock,
discard the entry and require a fresh request after unlock.

On a private polkitd or fixture bus test two eligible identities, success,
a wrong password, both user and authority cancellation, disappearance of the
requester and a lock during password entry. Verify the selected identity,
exactly one response to each request, no privileged action after cancellation,
and that the locker alone receives input. Repeat Settings’ protected timezone
change through GDM under joe enforcement. Screen-reader checks cover the action,
identity chooser, errors and masked entry. The authentication agent runs inside the panel.

**Agreement with mundane-apps.** At the suite's Terminal milestone, agree on
semantic foreground/background, selection, error, accent and focus roles,
spacing, fonts, keyboard actions and a sample settings row with a labelled
control. Probe AT-SPI on that control immediately. Once the suite publishes
the crate and the sample, pin `mundane-ui` by full git revision in the panel
workspace and commit the lockfile. Keep GPUI and gpui_platform on Zed
`f4fe0be1f8c7974db07e01c5cd05bee0cf4eb8d9`, the AccessKit revision
selected with mundane-apps and gpui-shades. Check
Cargo's resolved graph for a single GPUI source and run the shared sample on
Wayland on each pin update. The suite owns widgets and `mundane-services`
client adapters. Settings owns its data model and operations. At the Files
milestone exchange desktop IDs, MIME inputs and success/cancel/error fixtures
for the M6 portal tests. [Application suite plan](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/main/docs/plan.md)

Use desktop GSettings preferences for light/dark, high contrast, accent,
font/text scale and animation preference where supported. GTK/libadwaita use
their native styles. Map the same preferences to semantic roles in
`mundane-ui` and the shell. Read standard appearance keys from the Settings
portal and subscribe to changes. Verify which keys joe's GTK backend exports.
Fill any missing required keys in the desktop Settings backend using the same
preference source. Resolve system fonts through fontconfig and icons through
the installed icon theme. When reduced motion is enabled, settle all active
springs to their targets, skip zoom/slide and finish any crossfade in the next
frame. [Portal appearance settings](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html)

Build in keyboard traversal, visible focus, labelled controls, roles, values
and accessible actions for the launcher, notifications, menus, Settings and
locker. Use the pinned Linux AccessKit bridge and the suite’s shared
`mundane-ui` control contract. Give controls stable identities independent
of their labels. Expose toggle states and range values/actions, scope dialog
focus and restore it on dismissal. Route accessible and keyboard activation
through the same command. Test editable text, caret and selection through
AT-SPI, including IME and masked values. Inspect the live GTK and GPUI trees
on private buses and check Orca’s speech for a Settings toggle. [Shared control contract and release tests](https://github.com/theJoeLinuxFoundation/gpui-shades/blob/52a66ecacb0ed42285565ebc70e7e0e038ce20ae/docs/linux-accessibility.md#shared-control-contract-for-mundane-ui)
Provide a keyboard path to enable accessibility at login and retain it after
lock and logout.

Implement the Wayland keyboard monitor that libatspi discovers on the session
bus. The compositor owns `org.freedesktop.a11y.Manager` and exports
`org.freedesktop.a11y.KeyboardMonitor` at `/org/freedesktop/a11y/Manager`.
Orca uses the matching printable key event to distinguish typing from an edit
made by the application. Restrict monitoring to the authorized accessibility
service. Bind the authorization to its D-Bus connection, deliver events only
to that connection on the active seat, and revoke the watch when the owner
disconnects or the session ends. An ordinary app must be unable to acquire a
watch by claiming Orca’s bus name or receive key events by subscribing to
signals. [libatspi device selection](https://github.com/GNOME/at-spi2-core/blob/2.62.1/atspi/atspi-device.c#L246) · [Sway keyboard monitor](https://github.com/swaywm/sway/pull/9331)

**Acceptance.** Open a GTK/libadwaita app, Settings and a GPUI suite app side
by side. Change light/dark, accent, high contrast, text scale and reduced
motion. Check live updates, legibility, focus rings and lack of clipping at
100%, 150% and 200%. Toggle reduced motion mid-snap and mid-fade. With the
pointer unused, launch an app, adjust a setting, dismiss a dialog and unlock.
Orca must announce the focused role, name, value and state changes, with
passwords masked. Inspect the AT-SPI tree and activate the control through
its accessible action. Restart the session and check persistence.

Run Orca with character echo enabled on mundaneDE with private session and
accessibility buses and a recording speech server. Focus a GTK entry containing
`Tokyo` and type `z` at the end. Require the insert event, the new value
`Tokyoz` and exactly one spoken “z”. In a fresh trial, have the application
append the same character to the same focused field without a keypress. Require
the same insert and value and zero spoken “z” responses. Repeat both trials in
a GPUI/AccessKit entry. Also attempt to watch and subscribe from an ordinary
session client and verify it receives no keys during the typing trial.
[GPUI speech fixture](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/59982d6/tests/wayland/accessibility.py)

### Implementation and packaging

Install Orca and its speech backend (Speech Dispatcher and a voice), AT-SPI2
2.62.1 or compatible, GSettings desktop schemas, dconf, fontconfig and the
desktop icon theme. Ship the Polkit and PolkitAgent typelibs, polkitd and its
distribution-provided authentication helper and PAM files. The unprivileged
panel registers for its own logind session on the system bus. Super+Alt+S
toggles the session-owned Orca process. The screen reader's unique session-bus
connection receives directed keyboard signals. Lock and shell password entry
suspend delivery. The disposable test overlay supplies Orca 51.1 and records
speech at its speech-server interface.

The desktop portal advertises `org.freedesktop.appearance` color-scheme,
accent-color, contrast and reduced-motion and forwards font-name,
text-scaling-factor and enable-animations in `org.gnome.desktop.interface`.
The GTK backend on beta1 supplies color-scheme and contrast. Install
`mundane.portal` and `mundane-portals.conf` so the frontend can merge both
backends. Install `50-mundane.gschema.override` alongside the desktop schemas
in `/usr/share/glib-2.0/schemas` and run the distribution’s
`glib-compile-schemas` trigger. The override selects light on a fresh mundaneDE
login. Existing user choices take precedence and GNOME keeps its own default.
Settings writes `color-scheme`, `accent-color` and `enable-animations`
in `org.gnome.desktop.interface` and `high-contrast` in
`org.gnome.desktop.a11y.interface`. The shell consumes the frontend's appearance
snapshot and change signals. Settings and suite apps use the same mundane-ui
portal subscription, semantic colours and GPUI motion preference. Settings also
reads font, text scale and the accent name from the desktop interface namespace.
[Shared implementation at the pinned suite revision](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/80eed70f87a44cd31743cef499b8d42df8f097d3/crates/mundane-ui/src/appearance.rs)

The headless joe beta1 gate changes the preferences in Settings while libadwaita
and the suite’s Terminal stay open. It observes native style properties, Terminal
grid and cursor colours, and the suite’s moving and stationary GPUI marker.
The same gate checks Orca speech and character echo, keyboard monitoring access,
real and fixture polkit authentication, display rollback and a fresh GDM login.

Settings lists connector modes through `mundane-de output list`. Test, Keep
and Revert use the compositor's 15-second layout trial. The socket requires
kernel-supplied credentials for the session user. On joe, System opens
`mundane-packages` as the logged-in user. [Headless checks](../tests/qemu/session/README.md#headless-settings-and-accessibility-qualification)

## 8. Qualify the default session

Begin once joe has packaged the [vendored desktop and session payload](#joe-packaging-handoff).
The maintainer owns the recipe, image and default selection. Keep GNOME as the
image default during qualification. Boot disposable copy-on-write copies of
the candidate disks with headless KVM. Run the installed `/usr` session as an
ordinary user with the image’s policy, services and suite packages. Exclude
`/usr/local` overlay installs, host-built binaries and fixture portal or dave
services from this gate.

| joe image test | Pass condition and evidence |
| --- | --- |
| Build and install | From the pinned archive and SHA-256, build both crates offline in joe. Compare the sealed image’s package manifest with the handoff above. Check executable/shared-library resolution, compiled schemas, GI imports, session units, portal XML/routing and PAM services before first login. Record desktop/suite/dave revisions, image deployment ID, kernel, Mesa, driver and display modes. |
| Clean and upgraded x86 | Extend `tests/vm/release.tomlx` / `make test-release` for first-account creation, GDM session selection and a second boot. Separately start with a supported previous joe desktop, prepare the real upgrade with dave and reboot into the packaged candidate. Verify the existing account, files, MIME associations and explicit appearance preferences survive. |
| Session and apps | Add a mundaneDE scenario alongside the GNOME rows of `tests/vm/desktop.tomlx` / `make test-desktop` in both logging and enforcing modes. Discover the user’s published Wayland display. Verify the bottom taskbar, Start, tray, desktop icons and window buttons, then the application matrix with GTK, suite apps, X11 clipboard in both directions and IBus conversion in Firefox and GTK. Open, edit and save a removable file through its handler. Play Firefox audio. |
| Displays, sharing and privacy | With two virtual outputs, test scale, mode rollback, snap and reduced motion. In Firefox start a PipeWire share, verify its pixels and indicator, exercise the stop paths in milestone 6 and confirm the lock screen stays private. Accept and cancel a screenshot. Keep QMP screendumps and portal/PipeWire journal evidence. |
| Seat, accessibility and authorization | Switch VTs, suspend and wake, unlock, log out twice and check fresh input on each login. Change appearance with GTK and a suite app running. Hear Orca read Start and a Settings toggle, echo a typed character and stay quiet for a programmatic edit. Authenticate a Settings change, cancel another and lock during a prompt. Check expected policy decisions against `make test-security` and review the beta1 helper and virtio-gpu observations in milestone 4. Select GNOME at GDM and verify the same user files and preferences. |
| Real deployments and notices | Extend `tests/vm/deployments.tomlx` / `make test-deployments` with a desktop login. Observe “Upgrade ready, restart to apply” once per prepared ID. On successful boot check that pending status clears. Exhaust boot attempts and check that the shell names the attempted and restored systems once per fallback event. Log in again to check deduplication, then choose an explicit rollback and verify it stays quiet. Retain the corresponding unprivileged JSON snapshots and screenshots. |

Attach pass/fail results, relevant journals and unresolved limitations to each
candidate. Retire individual GNOME apps only when the suite’s corresponding
workflow passes on the packaged image. [joe VM scenarios](https://github.com/theJoeLinuxFoundation/joe-linux/tree/5dc2ac8/tests/vm)

**Final gate.** Once automated qualification passes, the maintainer logs in to
the candidate session on a real x86 machine and completes the [manual test](manual-test.md).
Record the image and revisions, hardware, automated results, observations and
any blocking issues. Fix failures and retest the candidate before recording the
maintainer’s dated sign-off. Only then may joe Linux change the default session.
Extend the checklist as each milestone lands.

## How to collect test evidence

| Where | Evidence to keep |
| --- | --- |
| Rust and service tests | Run `cargo test --locked` in the root and `cargo test --locked --workspace` in `settings-panel/`. Extend the current spring, snap, wallpaper, parser and TOML tests with clock-driven final frames, output transforms, focus transitions and config migration. Use private D-Bus fixtures for disappearing devices, cancellation and denied authorization. |
| Nested and headless tests | Start a disposable host compositor, private runtime/config/data directories and a session bus. Wait for socket and service readiness with a timeout. Drive real Wayland processes with input delivered by the harness. Record registry, configure/ack, focus and client exit. Keep virtual input confined to the test session. Supplement these tests with GPUI headless widget tests. |
| Screenshot checks | Pin the Mesa software renderer, fonts, locale, fixture data, geometry, scale, wallpaper, cursor and clock. Capture at a named state or spring tick, including the frame after settling. Compare geometry exactly and use a documented per-channel tolerance for pixels. On failure save reference, actual, diff and logs. Review changed reference images with the feature that changed them. |
| DRM, VM and hardware | Run the diskless two-output QEMU test for modes, mixed refresh, presentation and hotplug. Use the interactive VM for investigation and add joe image scenarios through the distribution handoff. The final maintainer gate covers physical devices, VT ownership, lock and suspend. Keep timing traces with hardware details. [QEMU tests](../tests/qemu/README.md) |
