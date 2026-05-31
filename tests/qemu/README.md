# Manual QEMU testing for the DRM/KMS backend

The DRM/KMS backend (`cargo run`, no `--winit`) needs a real seat/TTY and a DRM device, so
it can't run in a normal desktop session. This directory boots mundaneDE inside a throwaway
QEMU NixOS VM so you can exercise the production rendering path by hand.

This is a **manual** aid — there are no automated assertions. You boot the VM, run the
compositor, and watch it draw.

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

To quit the compositor: `Super+Shift+Q` or `Esc`. To shut the VM down: `poweroff`.

## Keybindings

| Keys | Action |
|------|--------|
| `Super+Return` | spawn a terminal (`foot`) |
| `Super+W` | reload / crossfade the wallpaper |
| `Super+,` | open `mundane-settings` (override the binary with `$MUNDANE_SETTINGS`) |
| `Super+Q` | close the focused window |
| `Super+Shift+Q` or `Esc` | quit the compositor |
| `Super`/`Alt` + left-drag | move a window; on release it springs to a magnetic snap |

## Seeing a wallpaper

A wallpaper only appears if you point the config at an image. The repo ships a test image
(`pexels-jplenio-2080960.jpg`), shared into the VM at `/mundane`. Create the config inside
the guest:

```bash
mkdir -p ~/.config/mundane
cat > ~/.config/mundane/config.toml <<'EOF'
[wallpaper]
path = "/mundane/pexels-jplenio-2080960.jpg"
mode = "fill"   # fill | fit | stretch | center | tile
EOF
mundane
```

You should see the image **spring-fade in** behind the windows. `Super+W` re-runs the fade
as a crossfade. Without a `path`, you get the dark clear colour instead.

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
