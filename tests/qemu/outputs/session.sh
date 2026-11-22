# PID 1 in the diskless test guest.
set -eu
trap 'sync; echo 1 >/proc/sys/kernel/sysrq; echo b >/proc/sysrq-trigger' EXIT
mount -t tmpfs tmpfs /tmp
mount -t tmpfs tmpfs /run
mkdir -p /dev/shm /run/udev /home/test /etc/fonts/conf.d /extra /merged
mount -t tmpfs tmpfs /dev/shm
if mount -t 9p -o trans=virtio,version=9p2000.L,ro,msize=1048576 extra /extra 2>/dev/null; then
    modprobe overlay
    mount -t overlay overlay -o lowerdir=/extra:/usr /merged
    mount --move /merged /usr
fi
export PATH=/usr/bin:/test LC_ALL=C.UTF-8 LIBSEAT_BACKEND=seatd SEATD_VTBOUND=0
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
export HOME=/home/test XDG_RUNTIME_DIR=/run/test XDG_CONFIG_HOME=/home/test/config
export XDG_STATE_HOME=/home/test/state XDG_CACHE_HOME=/home/test/cache
export GTK_A11Y=none GSETTINGS_BACKEND=memory GDK_BACKEND=wayland
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent
export FONTCONFIG_FILE=/etc/fonts/fonts.conf
cat >"$FONTCONFIG_FILE" <<'EOF'
<fontconfig><dir>/usr/share/fonts</dir><cachedir>/home/test/cache/fonts</cachedir></fontconfig>
EOF
printf '%s\n' root:x:0:0:root:/root:/bin/bash test:x:1000:1000:test:/home/test:/bin/bash >/etc/passwd
printf '%s\n' root:x:0: video:x:44: input:x:45: render:x:46: test:x:1000: >/etc/group
cat /proc/sys/kernel/random/uuid | tr -d - >/etc/machine-id
mkdir -pm700 "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME/mundane"
printf '[displays.Virtual-2]\nscale=1.5\n' >"$XDG_CONFIG_HOME/mundane/config.toml"
chown -R 1000:1000 "$XDG_RUNTIME_DIR" /home/test
/usr/lib/systemd/systemd-udevd --daemon
modprobe virtio_gpu
modprobe virtio_input
modprobe virtio_console
udevadm trigger
udevadm settle
seatd -g video >/tmp/seatd.log 2>&1 &
sleep .4
export RUST_LOG=info,mundane_present=debug
export NO_COLOR=1
setpriv --reuid=1000 --regid=1000 --groups=44,46 /test/mundane-de >/tmp/compositor.log 2>&1 &
for ((i=0;i<100;i++)); do [[ -S /run/test/wayland-1 ]] && break; sleep .1; done
export WAYLAND_DISPLAY=wayland-1
python /test/checks.py || { rg -a "hit |key |wl_pointer.*(enter|motion|button)|preferred_scale|create_buffer|create_immed" /tmp/gtk.log | tail -35; tail -35 /tmp/compositor.log | cut -c1-400; exit 1; }
echo "KMS TEST PASSED"
