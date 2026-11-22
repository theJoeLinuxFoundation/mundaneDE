#!/bin/sh
# Start a fresh headless guest for automated qualification.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
cache=${XDG_CACHE_HOME:-"$HOME/.cache"}/mundane-login
mkdir -p "$cache"
run=$(mktemp -d "$cache/run.XXXXXX")
PYTHONPATH="$here" python3 -c 'import sys; from machine import boot; boot(sys.argv[1],sys.argv[2])' "${1:-$HOME/Projects/joe-linux/build/images/joe-desktop-beta1.img}" "$run"
printf '%s\n' "$run"
