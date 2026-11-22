#!/bin/sh
# Xwayland binds both filesystem and Linux abstract X11 sockets.
set -eu
root=$(CDPATH='' cd -- "$(dirname "$0")/../.." && pwd)
terminal=$(command -v mundane-terminal) || { echo "mundane-terminal is required on PATH" >&2; exit 1; }
files=$(command -v mundane-files) || { echo "mundane-files is required on PATH" >&2; exit 1; }
mkdir -p "$root/target/nested-tests"
exec bwrap --unshare-pid --unshare-ipc --unshare-net --unshare-uts --die-with-parent \
    --ro-bind /usr /usr --symlink usr/lib /lib --symlink usr/lib /lib64 \
    --symlink usr/bin /bin --ro-bind /etc /etc --dev /dev --proc /proc \
    --tmpfs /tmp --perms 01777 --dir /tmp/.X11-unix --dir /run \
    --ro-bind "$root" "$root" --bind "$root/target/nested-tests" "$root/target/nested-tests" \
    --ro-bind "$terminal" /opt/mundane-apps/mundane-terminal --ro-bind "$files" /opt/mundane-apps/mundane-files \
    --chdir "$root" --clearenv --setenv PATH "/opt/mundane-apps:${PATH}" \
    --setenv LD_LIBRARY_PATH "${LD_LIBRARY_PATH:-}" --setenv PYTHONDONTWRITEBYTECODE 1 \
    -- python3 tests/nested/run.py --everyday "$@"
