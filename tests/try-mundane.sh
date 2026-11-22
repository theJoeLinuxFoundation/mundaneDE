#!/bin/sh
# Run the public joe desktop image with an opt-in mundaneDE session.
set -eu
repo=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 "$repo/tests/qemu/session/try_session.py" "$@"
