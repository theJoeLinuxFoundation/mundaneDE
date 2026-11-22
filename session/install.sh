#!/bin/sh
# Install the compositor and the four binaries from the settings-panel workspace.
set -eu
prefix=${PREFIX:-/usr/local}
dest=${DESTDIR:-}
here=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
gui=${2:-"$here/../settings-panel/target/release"}
install -Dm755 "${1:?path to built mundane-de}" "$dest$prefix/bin/mundane-de"
for binary in mundane-session mundane-shell mundane-settings mundane-lock; do
    install -Dm755 "$gui/$binary" "$dest$prefix/bin/$binary"
done
install -Dm644 "$here/mundane-lock.pam" "$dest/usr/lib/pam.d/mundane-lock"
install -d "$dest/usr/share/wayland-sessions"
sed "s|@bindir@|$prefix/bin|g" "$here/mundane.desktop.in" > "$dest/usr/share/wayland-sessions/mundane.desktop"
for schema in 50-mundane.gschema.override org.joelinux.mundane.shell.gschema.xml; do
    install -Dm644 "$here/$schema" "$dest/usr/share/glib-2.0/schemas/$schema"
done
for unit in mundane-session.target mundane-ibus.service; do
    install -Dm644 "$here/$unit" "$dest$prefix/lib/systemd/user/$unit"
done
install -Dm644 "$here/mundane-mimeapps.list" "$dest$prefix/share/applications/mundane-mimeapps.list"
install -Dm644 "$here/mundane-portals.conf" "$dest$prefix/share/xdg-desktop-portal/mundane-portals.conf"
install -Dm644 "$here/mundane.portal" "$dest$prefix/share/xdg-desktop-portal/portals/mundane.portal"
