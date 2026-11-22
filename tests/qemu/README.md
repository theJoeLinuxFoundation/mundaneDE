# DRM/KMS tests in QEMU

## Automated output test

On an x86_64 host with KVM, run from the repository root. The harness uses
QEMU, bsdtar, a kernel image and matching modules under `/usr/lib/modules`
with virtio GPU, input, serial and 9p support. The guest shares the host's
`/usr` read-only and expects the Arch-style library layout, bash, seatd,
systemd-udevd, GTK 4/PyGObject, Mesa EGL, lavapipe and Firefox. Put
`wayland-info` and the suite's `mundane-files` on PATH. An optional
`--extra-usr DIRECTORY` adds a read-only package overlay in the guest.

```sh
cargo build --locked --bins --examples
python3 tests/qemu/outputs/boot.py --kernel "/usr/lib/modules/$(uname -r)/vmlinuz"
```

The diskless guest boots with a private seat, two virtio-gpu scanouts,
virtual input, no network and no graphical host window. The test toggles
the second connector inside the guest. QMP reads back each scanout and
supplies pointer and keyboard events. Compositor and applications run as
an ordinary guest user. Each invocation keeps its serial log and RGB
captures under `target/kms-*/`.

| Check | Observation |
| --- | --- |
| Mixed scale and refresh | Drag a GTK EGL window from 800×600 at 74.999 Hz and 100% to 1024×768 at 60.004 Hz and 150%, then try 200%. A 200×120 logical rectangle covers exactly 200×120, 300×180 and 400×240 pixels respectively. The text sample resolves more high-contrast glyph edges at each scale, one-pixel stripes retain contrast, clicks reach the expected logical coordinates and the edge popup remains usable. Firefox and GPUI Files negotiate all three scales and show their expected colours on both scanouts. |
| Layout recovery | Try a rotated mode, let its 15-second timer expire and check the previous scanout. Remove the focused display, type into the relocated window, reconnect and repeat placement with reduced motion. Confirm a negative logical origin after its first page flip and check it survives reconnect. |
| Presentation | Match client sequence and timestamps to the guest kernel’s DRM events, check a superseded commit is discarded, then sample both outputs during a shared wallpaper fade. The compositor honors the synchronized presentation policy when a client requests async. |

The release run on 2026-11-22 used QEMU 11.1.2 with KVM, four guest vCPUs,
3 GiB RAM, virtio-vga and llvmpipe. During the shared fade, Virtual-1
returned 93 flip intervals with p50/p95/p99 of 13.334/13.334/13.334 ms.
Virtual-2 returned 74 intervals of 16.666/16.666/16.666 ms. To use the release
build, build with `--release` and pass `--binary target/release/mundane-de`.

The [nested timing test](../nested/README.md#frame-and-idle-budgets) measures
the release frame budget on a render node.

## Interactive NixOS guest

The DRM/KMS backend (`cargo run`, no `--winit`) needs a real seat/TTY and a DRM device, so
it can't run in a normal desktop session. This directory boots mundaneDE inside a throwaway
QEMU NixOS VM so you can exercise the production rendering path by hand.

## Prerequisites

- NixOS (or Nix with `<nixpkgs>` on `NIX_PATH`) — `run.sh` builds the VM with
  `nix-build '<nixpkgs/nixos>' -A vm`.
- KVM available (the dev shell bundles `qemu_kvm`). Software emulation also works, just
  slower.

## Usage

```bash
tests/qemu/run.sh
```

This will:

1. Build the compositor in `nix-shell` (`cargo build`).
2. Build the QEMU VM image from [`vm.nix`](./vm.nix).
3. Boot the VM. It auto-logs in as `root` on tty1 and prints usage.

Inside the guest:

```text
mundane              # run the DRM backend
RUST_LOG=debug mundane
```

The repository is shared into the VM at `/mundane`, and `mundane` runs
`/mundane/target/debug/mundane-de` with the right `LD_LIBRARY_PATH`, `XKB_CONFIG_ROOT`, and
`LIBSEAT_BACKEND=seatd`. Rebuild on the host (`cargo build`) and just re-run `mundane` in the
guest — no VM rebuild needed.

To quit the compositor: `Super+Shift+Q`. To shut the VM down: `poweroff`.

## Keybindings

| Keys | Action |
|------|--------|
| `Super+Return` | spawn a terminal (`foot`) |
| `Super+W` | reload / crossfade the wallpaper |
| `Super+,` | open `mundane-settings` (override the binary with `$MUNDANE_SETTINGS`) |
| `Super+Q` | close the focused window |
| `Super+Shift+Q` | quit the compositor |
| `Super`/`Alt` + left-drag | move a window; on release it springs to a magnetic snap |

## Seeing a wallpaper

The compositor displays the built-in Afterlight wallpaper on startup. Press
`Super+W` to replay the crossfade. To try another image, set `[wallpaper].path`
in the guest's `~/.config/mundane/config.toml`. Use an absolute path visible to the
guest. The host checkout is available at `/mundane`.

## Troubleshooting

- **`no DRM GPU found` / `no connected output`** — the VM didn't expose a usable KMS device.
  Confirm `/dev/dri/card0` exists in the guest (`ls /dev/dri`). The default QEMU VGA exposes
  a `bochs-drm` KMS device; ensure `hardware.graphics.enable` stuck.
- **`libseat`/session errors** — check `systemctl status seatd` in the guest; `mundane` sets
  `LIBSEAT_BACKEND=seatd`.
- **GL/EGL import failures** — rendering uses software GL (llvmpipe) in the VM, which is slow
  but should work. `RUST_LOG=debug mundane` shows the renderer/EGL setup.
- **Linker errors at launch** — the guest must build from the *same* `<nixpkgs>` as the host
  `nix-shell`, since the host-built binary resolves its libraries from the shared store via
  `LD_LIBRARY_PATH`. If they diverge, rebuild both against the same channel.
