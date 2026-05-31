#!/usr/bin/env bash
#
# Build the mundaneDE DRM/KMS backend and boot it in a QEMU NixOS VM for manual testing.
# See README.md for prerequisites and what to expect.
#
# Usage: tests/qemu/run.sh [extra args passed to the QEMU runner]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

echo "==> Building the compositor (debug) in nix-shell"
( cd "$REPO" && nix-shell --run "cargo build" )

echo "==> Building the test VM (this evaluates <nixpkgs/nixos>)"
vm_out="$(nix-build '<nixpkgs/nixos>' \
  -A vm \
  -I nixos-config="$HERE/vm.nix" \
  --no-out-link)"

runner=("$vm_out"/bin/run-*-vm)
if [ ! -x "${runner[0]}" ]; then
  echo "error: could not find the VM runner under $vm_out/bin" >&2
  exit 1
fi

echo "==> Booting VM — log in is automatic; run 'mundane' inside. 'poweroff' to exit."
exec "${runner[0]}" "$@"
