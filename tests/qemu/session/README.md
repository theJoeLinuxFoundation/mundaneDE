# GDM session test on joe Linux x86

Use joe’s beta1 `--test` desktop image with its serial test console and GDM
autologin account. From the repository root on an x86 KVM host, build the
compositor and the settings-panel workspace. Supply a logind-enabled libseat compatible with the guest. The shell registers the polkit agent. The host and guest used glibc 2.44.

```sh
cargo build --release --locked --bins --examples
cargo build --release --locked --workspace --manifest-path settings-panel/Cargo.toml
run=$(tests/qemu/session/boot.sh)
python3 tests/qemu/session/prepare.py "$run" \
    --libseat /usr/lib/libseat.so.1 \
    --settings settings-panel/target/release/mundane-settings
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

The launcher builds a private export of freshly fetched `origin/main`. Its terminal
output and QEMU title show the full revision. Both HEAD and any explicit
`--revision COMMIT` must be published on an origin branch or tag. Use
`--from-working-tree` to opt into a dirty checkout or local commits. Headless trials default to a separate qualification directory. On the first start it installs the build in a
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
libseat, binutils, curl and bsdtar.
The helper uses installed grim and ibus-wayland when available and can extract
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

QMP captures the scanouts and delivers seat input. This gate covers server-side window buttons, idle inhibition, workspace moves, shell replacement, per-window activation, a StatusNotifierItem, a notification action, Wi-Fi and audio routing between two guest sinks. The dedicated feedback and lock drivers below cover dave events and multi-output PAM locking.

### Headless menu retest

On a freshly booted, dedicated 1920×1080 headless guest, run
`python3 tests/qemu/session/qualify_menus.py "$MUNDANE_TRY_DIR"`. The test
drives the virtual pointer and keyboard, counts actual clicks and keystrokes
in a GTK client and activates a tray menu action. The separate Settings driver checks Wi-Fi, notifications, PipeWire volume and virtual laptop devices. QMP samples the workspace transition
with animations enabled and with reduced motion. [Comparison screenshots](../../../docs/screenshots/shell/README.md)

### Daily desktop parity

On a fresh headless 1920×1080 login, run the following from the repository
root. Use a dedicated overlay. The test opens applications, changes the test
account’s favourites, restarts the panel and resets the favourites preference.
Install Netpbm on the host for lossless QMP-to-PNG conversion.

```sh
python3 tests/qemu/session/qualify_parity.py "$MUNDANE_TRY_DIR" target/parity-screenshots
```

[Historical comparison and images](../../../docs/parity.md)

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
also exercises screenshot denial, the GPUI file chooser, the preferred URI
handler and a round trip from a shell notification button to its caller.

### Headless Settings and accessibility qualification

Use a dedicated, prepared, headless joe overlay with GDM logged in to mundaneDE.
The driver changes the guest's reader profile, accessibility preferences and
display settings. It uses QMP to send keys and capture rendered frames.

```sh
suite_rev=78540788819e4c899fa4a15df0f1d5ec1cefa938
suite=$(mktemp -d)
git -C "$HOME/Projects/mundane-apps" archive "$suite_rev" | tar -x -C "$suite"
cargo +stable build --locked --manifest-path "$suite/Cargo.toml" -p mundane-terminal
cargo +stable build --locked --manifest-path "$suite/Cargo.toml" -p mundane-ui --example motion
cargo +stable build --locked --manifest-path settings-panel/Cargo.toml --example echo
python3 tests/qemu/session/qualify_accessibility.py "$run" \
    --echo settings-panel/target/debug/examples/echo --suite-target "$suite/target/debug"
```

Each typing trial starts a fresh Orca input history, focuses a real GTK entry
or mundane-ui text field, and compares AT-SPI insert offsets, final value and
recorded speech. A second ordinary client checks directed-signal isolation.
The desktop checks use the same GDM session for launcher and toggle speech,
live libadwaita preferences, the suite Terminal’s rendered surface and cursor,
the suite’s reduced-motion preview, display rollback and a PAM-protected
timezone change. The two-identity fixture uses a separate runtime directory
and authority bus inside the guest.

### GPUI shell step 1

Build all five release binaries from the repository root. Create a fresh 1920×1080 headless guest with three available virtual connectors and the frozen beta1 image. The trace flag enables layer-commit and DRM redraw timestamps in the normal mundaneDE session.

```sh
cargo build --release --locked --bin mundane-de --example nested-capabilities
cargo build --release --locked --workspace --manifest-path settings-panel/Cargo.toml
cache="${XDG_CACHE_HOME:-$HOME/.cache}/mundane-login"
mkdir -p "$cache"
run=$(mktemp -d "$cache/gpui-step1.XXXXXX")
PYTHONPATH=tests/qemu/session python3 -c 'import sys; from machine import boot; boot(sys.argv[1], sys.argv[2], large=True)' \
  "$HOME/Projects/joe-linux/build/images/joe-desktop-beta1.img" "$run"
driver=tests/qemu/session
python3 "$driver/prepare.py" "$run" --libseat /usr/lib/libseat.so.1 \
  --settings settings-panel/target/release/mundane-settings \
  --trace-frames
python3 "$driver/qualify_parity.py" "$run" "$run/screenshots"
python3 "$driver/guest.py" "$run" shell 'systemctl restart gdm'
python3 "$driver/qualify_menus.py" "$run" --screenshots "$run/screenshots"
python3 "$driver/qualify_gpui_outputs.py" "$run"
python3 "$driver/qualify_gpui_budget.py" "$run"
python3 "$driver/guest.py" "$run" shell 'sync && systemd-run --on-active=1s systemctl poweroff'
```

Use a fresh login for the menu input probe. Its real seat clicks, focus and
scanout checks assume an otherwise empty first workspace. The output test
reads both physical QMP scanouts at independent scales. The budget test
starts GDM repeatedly in this overlay, measures a cold user cache separately
and records 20 warm logins, idle CPU, PSS, clock updates and redraws.

### GPUI shell step 2

Use the headless GPUI guest prepared above. Build the real Terminal
and motion probe from the same mundane-apps revision as the desktop. In this
example `suite` points to the mundane-apps revision in `settings-panel/Cargo.toml`. Its build output stays in
the test run directory.

```sh
export CARGO_TARGET_DIR="$run/suite-target"
cargo build --manifest-path "$suite/Cargo.toml" --locked --release -p mundane-terminal
cargo build --manifest-path "$suite/Cargo.toml" --locked --release -p mundane-ui --example motion
python3 "$driver/qualify_gpui_settings.py" "$run" "$run/screenshots" \
  --suite-target "$CARGO_TARGET_DIR/release"
```

The driver uses QMP to exercise all three pin gestures and sample the power
icons and scaled output. It changes a virtual NetworkManager radio and a real
PipeWire sink. The laptop case temporarily starts the supervised guest shell
on a private system bus with BlueZ, UPower and logind fixtures and a private
backlight mount. It restores the shell and logs in again. Use a dedicated
test overlay. The remaining checks drive native Settings forms, observe live
appearance in GTK, Terminal and the taskbar, and record Orca’s speech. Follow
with the parity, menu, multi-output and budget drivers above.


### GPUI shell step 3

Prepare the headless GPUI preview as in step 1 and install the wlr and grim versions from the portal test above. Use a dedicated overlay for each driver or start each with a fresh GDM login. The feedback driver temporarily interposes a dave status fixture for the supervised shell and restores the executable when finished.

```sh
python3 "$driver/qualify_gpui_feedback.py" "$run" "$run/screenshots"
python3 "$driver/qualify_portals.py" "$portal_run" --screenshots "$portal_run/screenshots"
```

The feedback driver checks notification ownership, actions, expiration, bounded history, Do Not Disturb, typing through a volume OSD and persistent dave notice IDs. The step 2 laptop and Orca cases also check a brightness key on a virtual keyboard and speech for a notification in history. Use mundane-apps at the desktop’s pin for the live Terminal test.

The portal driver compares an occluded window and a second monitor against the pixels delivered through the public portal. It checks cancellation, Stop on both taskbars, window close, browser exit, hot-unplug, lock and loss of either service. It changes the monitor pattern on a live stream and again after revocation, checks the PipeWire graph and worker exit, and observes an appearance change through the recovered frontend. It also opens and cancels the GPUI file chooser, follows an associated URI and invokes a notification action. The budget driver measures the idle session with all three backends registered.

### GPUI shell step 4

Boot mundaneDE in a dedicated headless joe guest with the accessibility dependencies installed. Use the GPUI echo example and the suite's Terminal and motion example from the pinned workspace, as in the milestone 7 accessibility test.

```sh
python3 "$driver/qualify_accessibility.py" "$run" --echo "$echo_binary" --suite-target "$suite_target"
python3 "$driver/qualify_gpui_budget.py" "$budget_run"
```

The first driver records Orca's own speech requests on the virtual seat. It checks typed and programmatic inserts, a delivered notification, a password entry, session restart and the active GDM login. A real Settings timezone change goes through joe's polkitd, helper and PAM. A second authority on a private bus drives the installed agent through a two-administrator conversation, cancellation, lock, authority disappearance and logout. QMP records the empty prompt in three appearances. Use a separate trace-enabled overlay for the budget and run its startup samples with the host otherwise idle. `--startup-only` repeats the cold and 20 warm logins independently.

### GPUI shell step 5

Build and install the workspace with the step 1 procedure. The installer takes the sibling `mundane-lock` executable and installs its PAM service. On a dedicated enforcing guest, run the step 4 accessibility driver and the step 3 portal driver in the GPUI session. The accessibility driver captures the authentication dialog in three appearances and compares the password field’s scanout position on a denied retry with animations enabled and disabled. The portal driver checks that Firefox stops receiving shared pixels on lock.

Run the lock driver last on its own overlay. It temporarily moves the locker executable, terminates the session’s shell and locker, and adds a denying account rule to the guest’s PAM service. It restores the service, binary and display configuration on exit. The driver uses the guest’s serial shell and QMP, with three virtio outputs available. Reboot before mounting the 9p share again after the S3 test. On beta1 a new 9p mount after wake can hang in the kernel.

```sh
python3 "$driver/qualify_gpui_lock.py" "$run" "$run/locker-screenshots"
```

The driver checks whole scanouts, monitors delivery to a background client, examines the protected AT-SPI field and records Orca’s speech. It verifies both wrong-password and PAM account denial, fresh conversations after VT and S3, and actual focus restoration after success. The compositor’s Rust tests also exercise public, shell, input-method and replacement locker connections, replayed tickets, a queued completion from a closed peer and obsolete generations. [Protocol and measurements](../../../docs/shell-gpui.md#step-5-gpui-lock)

### GPUI shell step 6

Use the ordinary headless mundaneDE overlay described above. The guest needs Nautilus, Text Editor, Firefox, the IBus daemon, its Python GI typelib and standalone `ibus-wayland`. Frozen beta1 omits the Wayland frontend, so supply a compatible upstream binary in the overlay. The desktop driver registers its own conversion engine for the session. The portal grant test needs Flatpak, Bubblewrap, the document portal and the guest’s own dynamic libraries. It constructs and removes a tiny local Flatpak and runtime without using an external app remote.

```sh
cc -O2 "$driver/portals/grant.c" -o "$run/file-grant" $(pkg-config --cflags --libs gio-2.0)
python3 "$driver/qualify_gpui_desktop.py" "$run" "$run/step6-screenshots"
python3 "$driver/qualify_portals.py" "$run" --screenshots "$run/step6-screenshots" --flatpak-client "$run/file-grant"
python3 "$driver/qualify_gpui_lock.py" "$run" "$run/step6-screenshots"
```

Use a clean login between drivers. Run the lock driver last because it suspends the guest. It uses ImageMagick on the host to compare the virtual scanouts with the expected dimmed wallpaper. The desktop driver exercises live directory updates, focus restoration and real default handlers, followed by Japanese conversion in two GTK inputs and a Firefox form. The portal driver checks both the chosen URI and the data read from the Flatpak’s document grant.
