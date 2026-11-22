#!/bin/sh
# Boot a joe --test image on a disposable overlay. Print the private run directory.
set -eu
image=$(realpath "${1:?path to joe-desktop.img}")
firmware=${OVMF_DIR:-/usr/share/edk2/x64}
cache=${XDG_CACHE_HOME:-"$HOME/.cache"}/mundane-login
mkdir -p "$cache"
run=$(mktemp -d "$cache/run.XXXXXX")
cd "$run"
mkdir share
cp "$firmware/OVMF_VARS.4m.fd" vars.fd
qemu-img create -q -f qcow2 -F raw -b "$image" system.qcow2
stat -c '%s %Y %Z %i' "$image" > backing.stat
# Explicit read-only raw and file nodes keep even the base descriptor read-only.
env -i PATH="$PATH" HOME="$run" qemu-system-x86_64 -daemonize -pidfile vm.pid \
  -machine q35,accel=kvm -cpu host -m 6G -smp 8 -global ICH9-LPC.disable_s3=0 \
  -nodefaults -display none -nic none -monitor none -device virtio-vga \
  -device virtio-keyboard-pci -device virtio-tablet-pci \
  -drive "if=pflash,format=raw,readonly=on,file=$firmware/OVMF_CODE.4m.fd" \
  -drive if=pflash,format=raw,file=vars.fd \
  -blockdev "driver=file,node-name=basefile,read-only=on,filename=$image" \
  -blockdev driver=raw,node-name=base,read-only=on,file=basefile \
  -blockdev driver=file,node-name=layerfile,filename=system.qcow2 \
  -blockdev driver=qcow2,node-name=layer,backing=base,file=layerfile \
  -device virtio-blk-pci,drive=layer \
  -virtfs local,path=share,mount_tag=mundane,security_model=none,readonly=on \
  -chardev socket,id=console,path=serial,server=on,wait=off,logfile=serial.log \
  -serial chardev:console -qmp unix:qmp,server=on,wait=off
printf '%s\n' "$run"
