# GDM session test on joe Linux x86

Use joe’s beta1 `--test` desktop image with its serial test console and GDM
autologin account. From the repository root on an x86 KVM host, build the
compositor and Settings. Supply a logind-enabled libseat compatible with the guest. The harness stages GTK 4 layer-shell and swaylock in the overlay. The shell registers the polkit agent. The host and guest used glibc 2.44.

```sh
cargo build --release --locked --bins --examples
cargo build --locked --manifest-path settings-panel/Cargo.toml
run=$(tests/qemu/session/boot.sh)
python3 tests/qemu/session/prepare.py "$run" \
    --libseat /usr/lib/libseat.so.1 \
    --settings settings-panel/target/debug/mundane-settings
python3 tests/qemu/session/qualify.py "$run"
python3 tests/qemu/session/guest.py "$run" qmp system_powerdown
```

The scripts create a private run directory in `$XDG_CACHE_HOME/mundane-login`
(default `~/.cache/mundane-login`). QEMU opens both backing nodes read-only.
The qcow2 overlay and the writable firmware variables live in that directory.
The headless q35 guest has 6 GiB RAM, eight vCPUs, virtio VGA, keyboard and
tablet, and a read-only 9p mount containing only staged files. It has no
network or host seat devices. OVMF comes from `/usr/share/edk2/x64` or
`OVMF_DIR`. QMP sends every key and captures the scanout. The serial driver
uses the guest's `joe# ` prompt.

Preparation waits for joe’s first-boot sealing, installs into the overlay,
selects `mundane` through AccountsService and boots the existing EFI entry with
`joe=enforce`. Qualification requires that enforcement is active. Use a fresh
run for each qualification.

* A D-Bus-activated GTK client records the display it connected to and its
  session ID. QMP sends a key and pointer click. A unique colour on scanout
  proves the client repainted.
* Ctrl+Alt+F6 switches away. The guest checks the active VT and released
  input descriptors. After switching back, the same client receives input and
  changes the pixels. ACPI S3 must reach QEMU’s suspended state. After
  `system_wakeup`, another key produces a different frame.
* Super+, opens the real GPUI Settings application. The test selects
  Africa/Cairo on the Date & Time page, cancels the polkit dialog and checks
  that UTC remains selected. It repeats the change, authenticates using the
  test account and verifies the timezone through timedated.
* Super+Shift+Q logs out to GDM. The test checks the old clients, agent,
  Wayland and Xwayland control sockets, user targets and activation values.
  At the greeter, restarting GDM rearms the image’s autologin. Each of three
  mundane logins must open a new client with working seat devices. The final
  GDM login selects GNOME through AccountsService.

The beta1 reference run completed three mundaneDE logins and three logouts,
then selected GNOME Shell 51.1 on the active Wayland seat. The two backing
nodes remained read-only and the source image retained its size, inode, mtime
and ctime. See the [packaging handoff](../../../docs/plan.md#4-enter-and-leave-a-real-session)
for the guest journal observations under enforcement and across suspend.

The serial transcript, firmware variables, overlay and named PPM screenshots
remain in the private run directory. The overlay contains the test account and
its session state.

## Try the desktop

Run this command from the repository root.

```sh
./tests/try-mundane.sh
```

The launcher requires a clean checkout, fetches and prints the origin/main revision, and builds a private export. Use `--revision COMMIT` for a specific revision or `--from-working-tree` to opt into development changes. Headless trials default to a separate qualification directory. On the first start it installs the build in a
private qcow2 overlay, selects mundaneDE for joe through GDM and checks the
enforcing boot headlessly. It then opens the QEMU window. The guest has eight
KVM vCPUs, 6 GiB RAM and a 1920×1080 primary display. The test account and
password are both `joe`. Open Start with Super+Space, move between workspaces
with Super+1 through Super+4, focus the desktop icons with Super+D and lock
with Super+L. The clock and Super+S open Quick Settings. Choose Shut Down and confirm to end the run cleanly.

The default base is `~/Projects/joe-linux/build/images/joe-desktop-beta1.img`.
Both base nodes and the staging share are read-only. Guest writes and EFI
variables persist in `~/.cache/mundane-try/desktop` (under `XDG_CACHE_HOME`
when set). Relaunching reuses that disk and updates the installed shell when
the checkout changes. An image replacement requires a fresh `MUNDANE_TRY_DIR`.
Select a different compatible beta1 test image with `--image PATH`.
The guest has no network interface or host device passthrough.

The reference x86 host supplies KVM, QEMU with GTK display support, OVMF,
rustup stable and the compositor and Settings build dependencies. Install
libseat, GTK 4 layer-shell and its GI typelib, binutils, curl and bsdtar.
The helper uses installed grim and swaylock when available and can extract
checksum-verified binaries from the Arch package archive. The beta1 overlay
also receives Orca 51.1 and python-dasbus for screen-reader tests.

The harness builds xdg-desktop-portal-wlr 0.9.0 from a checksum-verified
release source tree in its cache. The host needs a C compiler, pkg-config,
the Wayland scanner and protocols, libinih, GBM, libdrm, PipeWire and systemd
development files, plus libjpeg.so.8 for grim. Use Meson and Ninja, or uvx
to run the pinned build tools. In the guest, media and permissions run
against joe’s own PipeWire, WirePlumber and desktop portal frontend.

### Headless shell qualification

Use a dedicated guest because the test controls its virtual devices, logs in
and out of the locker and temporarily substitutes shell-only dave fixtures.
Run these commands from the repository root.

```sh
export MUNDANE_TRY_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/mundane-try/qualification"
./tests/try-mundane.sh --headless
cargo +stable build --locked --release --example nested-capabilities
python3 tests/qemu/session/qualify_shell.py "$MUNDANE_TRY_DIR"
python3 tests/qemu/session/guest.py "$MUNDANE_TRY_DIR" qmp system_powerdown
```

QMP captures the actual scanouts and delivers virtual seat input. Guest AT-SPI
activates named controls. The gate exercises Start, desktop files, server-side
window buttons, workspace moves, panel restart, app notifications, a
StatusNotifierItem, virtual Wi-Fi and audio devices,
three locked displays, locker loss, wrong and correct passwords and S3.
The beta1 image predates dave 0.4.0-r3. The panel reads schema-1 fixtures
through its normal unprivileged command, including repeated polls and shell
restarts. The actual deployment and failed-boot evidence belongs to joe’s
dave VM suite.

```sh
python3 tests/qemu/session/upgrade_checks.py
runtime=$(mktemp -d)
env -i HOME="$runtime" PATH=/usr/bin:/bin XDG_RUNTIME_DIR="$runtime" \
  dbus-run-session -- python3 tests/qemu/session/device_checks.py
```


### Headless menu retest

On a freshly booted, dedicated 1920×1080 headless guest, run
`python3 tests/qemu/session/qualify_menus.py "$MUNDANE_TRY_DIR"`. The test
drives the virtual pointer and keyboard, counts actual clicks and keystrokes
in a GTK client, activates a tray menu action and checks Wi-Fi, notifications
and PipeWire volume against the guest services. A GTK fixture supplies laptop
battery, backlight and Bluetooth states. QMP samples the workspace transition
with animations enabled and with reduced motion. [Comparison screenshots](../../../docs/screenshots/shell/README.md)

### Headless portal qualification

Boot a dedicated overlay from the frozen beta1 image. Run from the repository
root. The two virtual displays, browser profile, PipeWire graph and D-Bus
services used for capture are all inside the guest.

```sh
export MUNDANE_TRY_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/mundane-try/portal-qualification"
./tests/try-mundane.sh --headless
python3 tests/qemu/session/qualify_portals.py "$MUNDANE_TRY_DIR"
python3 tests/qemu/session/guest.py "$MUNDANE_TRY_DIR" qmp system_powerdown
```

The gate compares a portal PNG with QMP scanout samples and checks the
decoded Firefox video against two known colour bands. A contrasting window
covers the shared window on the first display. Stop, unplug, browser exit
and lock must remove the PipeWire node and the shell indicator. The gate
also exercises screenshot denial, the GTK file chooser, the preferred URI
handler and a round trip from a shell notification button to its caller.

### Headless Settings and accessibility qualification

Use a dedicated, prepared, headless joe overlay with GDM logged in to mundaneDE.
The driver changes the guest's reader profile, accessibility preferences and
display settings. It uses QMP to send keys and capture rendered frames.

```sh
cargo +stable build --locked --manifest-path settings-panel/Cargo.toml --example echo
python3 tests/qemu/session/qualify_accessibility.py "$run" \
    --echo settings-panel/target/debug/examples/echo
```

Each typing trial starts a fresh Orca input history, focuses a real GTK entry
or mundane-ui text field, and compares AT-SPI insert offsets, final value and
recorded speech. A second ordinary client checks directed-signal isolation.
The desktop checks use the same GDM session for launcher and toggle speech,
native libadwaita and GPUI colours, display rollback and a PAM-protected
timezone change. The two-identity fixture uses a separate runtime directory
and authority bus inside the guest.
