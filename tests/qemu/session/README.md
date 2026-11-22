# GDM session test on joe Linux x86

Use joe’s beta1 `--test` desktop image with its serial test console and GDM
autologin account. From the repository root on an x86 KVM host, build the
compositor and Settings. Supply a logind-enabled libseat and a standalone GTK
polkit authentication agent compatible with the guest. The reference run used
the host’s libseat.so.1 and polkit-gnome’s authentication agent. All other
runtime libraries came from joe. The host and guest used glibc 2.44.

```sh
cargo build --release --locked --bins --examples
cargo build --locked --manifest-path settings-panel/Cargo.toml
run=$(tests/qemu/session/boot.sh /path/to/joe-desktop.img)
python3 tests/qemu/session/prepare.py "$run" \
    --libseat /usr/lib/libseat.so.1 --agent /path/to/polkit-gnome-authentication-agent-1 \
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
