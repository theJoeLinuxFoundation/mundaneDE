#!/bin/sh
# Run headless Sway with a single DRM render node and the NVIDIA driver devices.
set -eu
root=$(CDPATH='' cd -- "$(dirname "$0")/../.." && pwd)
node=$(realpath "${1:?supply a DRM render node}"); shift
case "$node" in /dev/dri/renderD[0-9]*) test -c "$node";; *) exit 2;; esac
mkdir -p "$root/target/nested-tests"
exec bwrap --unshare-net --unshare-pid --unshare-ipc --unshare-uts --die-with-parent \
  --ro-bind /usr /usr --symlink usr/bin /bin --symlink usr/lib /lib64 --symlink usr/lib /lib \
  --ro-bind /etc /etc --ro-bind /sys /sys --proc /proc --dev /dev --tmpfs /tmp --dir /run \
  --ro-bind /dev/dri /dev/dri --ro-bind-try /run/udev/data /run/udev/data --dev-bind "$node" "$node" --dev-bind /dev/nvidiactl /dev/nvidiactl \
  --dev-bind /dev/nvidia0 /dev/nvidia0 --dev-bind-try /dev/nvidia-uvm /dev/nvidia-uvm \
  --ro-bind "$root" "$root" --bind "$root/target/nested-tests" "$root/target/nested-tests" \
  --chdir "$root" --clearenv --setenv PATH "$PATH" --setenv LD_LIBRARY_PATH "${LD_LIBRARY_PATH:-}" \
  --setenv PYTHONDONTWRITEBYTECODE 1 -- python3 tests/nested/run.py --gpu "$node" "$@"
