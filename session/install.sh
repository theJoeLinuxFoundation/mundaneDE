#!/bin/sh
# Install session integration for distro packaging or into a staging tree.
set -eu
prefix=${PREFIX:-/usr/local}
dest=${DESTDIR:-}
here=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
install -Dm755 "${1:?path to built mundane-de}" "$dest$prefix/bin/mundane-de"
install -Dm755 "$here/mundane-session" "$dest$prefix/bin/mundane-session"
install -Dm644 "$here/50-mundane.gschema.override" "$dest/usr/share/glib-2.0/schemas/50-mundane.gschema.override"
install -Dm644 "$here/org.joelinux.mundane.shell.gschema.xml" "$dest/usr/share/glib-2.0/schemas/org.joelinux.mundane.shell.gschema.xml"
install -d "$dest/usr/share/wayland-sessions"
sed "s|@bindir@|$prefix/bin|g" "$here/mundane.desktop.in" > "$dest/usr/share/wayland-sessions/mundane.desktop"
if [ -n "${2:-}" ]; then
    install -Dm755 "$2" "$dest$prefix/bin/mundane-shell"
    sed "s|@bindir@|$prefix/bin|g" "$here/mundane-gpui.desktop.in" > "$dest/usr/share/wayland-sessions/mundane-gpui.desktop"
fi
for unit in mundane-session.target mundane-ibus.service; do
    install -Dm644 "$here/$unit" "$dest$prefix/lib/systemd/user/$unit"
done
install -Dm644 "$here/mundane-portals.conf" "$dest$prefix/share/xdg-desktop-portal/mundane-portals.conf"
install -Dm755 "$here/mundane-panel" "$dest$prefix/bin/mundane-panel"
for script in "$here"/shell/*.py; do
    install -Dm644 "$script" "$dest$prefix/share/mundane/shell/$(basename "$script")"
done

install -Dm644 "$here/mundane.portal" "$dest$prefix/share/xdg-desktop-portal/portals/mundane.portal"
