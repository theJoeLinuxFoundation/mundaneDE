#!/usr/bin/env bash
# Export the committed desktop and vendor both independent Cargo lockfiles.
set -euo pipefail
export LC_ALL=C TZ=UTC
unset TAR_OPTIONS XZ_OPT XZ_DEFAULTS
umask 022

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
commit=$(git -C "$repo" rev-parse --verify HEAD)
epoch=${SOURCE_DATE_EPOCH:-$(git -C "$repo" show -s --format=%ct "$commit")}
[[ $epoch =~ ^[0-9]+$ ]] || { echo "Invalid SOURCE_DATE_EPOCH" >&2; exit 1; }

out="$repo/target/dist"
mkdir -p "$out"
work=$(mktemp -d "$out/.stage.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/source"
git -C "$repo" archive --format=tar "$commit" | tar -xf - -C "$work/source"
version=$(python3 -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["package"]["version"])' "$work/source/Cargo.toml")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][a-zA-Z0-9.+-]+)?$ ]] || { echo "Invalid package version" >&2; exit 1; }
name="mundaneDE-$version"
mv "$work/source" "$work/$name"
(
    cd "$work/$name"
    mkdir -p .cargo
    cargo vendor --locked --versioned-dirs --sync settings-panel/Cargo.toml vendor > .cargo/config.toml
)

tar -C "$work" --sort=name --format=gnu --mtime="@$epoch" --owner=0 --group=0 \
    --numeric-owner --mode='a=rX,u+w' -cf - "$name" |
    xz -2 --threads=2 --block-size=8MiB > "$work/$name.tar.xz"
mv -fT -- "$work/$name.tar.xz" "$out/$name.tar.xz"
sha256sum "$out/$name.tar.xz"
