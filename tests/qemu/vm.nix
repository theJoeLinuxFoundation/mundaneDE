# NixOS VM for manually testing the mundaneDE DRM/KMS backend under QEMU.
#
# Build & run with ./run.sh (see README.md). This boots a NixOS guest with a DRM/KMS
# device and seatd, auto-logs in as root on tty1, and provides a `mundane` command that
# runs the DRM backend against the repo's prebuilt debug binary. NixOS `build-vm` shares
# the host /nix/store into the guest, and we additionally share the repo at /mundane, so
# the host-built binary and its store-path dependencies resolve inside the VM.
#
# This is a manual-testing aid, not an automated test — boot it, run `mundane`, and watch.
{ config, pkgs, lib, modulesPath, ... }:

let
  # Runtime libraries the compositor links/dlopens — mirrors shell.nix so the host-built
  # binary (which relies on LD_LIBRARY_PATH, not rpath) finds the same libs in the guest.
  runtimeLibs = with pkgs; [
    wayland
    libxkbcommon
    libdrm
    mesa
    libgbm
    libinput
    systemd
    seatd
    libGL
    libdisplay-info
  ];

  mundane = pkgs.writeShellScriptBin "mundane" ''
    export LD_LIBRARY_PATH=${lib.makeLibraryPath runtimeLibs}
    export XKB_CONFIG_ROOT=${pkgs.xkeyboard_config}/share/X11/xkb
    export LIBSEAT_BACKEND=seatd
    export RUST_LOG=''${RUST_LOG:-info}
    if [ ! -x /mundane/target/debug/mundane-de ]; then
      echo "error: /mundane/target/debug/mundane-de not found — build it on the host first" >&2
      echo "       (run.sh does this for you via 'cargo build')" >&2
      exit 1
    fi
    exec /mundane/target/debug/mundane-de "$@"
  '';

  # Default config: turn the wallpaper on automatically so the demo looks finished, and so
  # the full-screen wallpaper render damages every frame's cursor-trail area cleanly even if
  # we ever regress the damage-tracking path for tiny moving elements like the pointer.
  mundaneConfig = pkgs.writeText "mundane-config.toml" ''
    [wallpaper]
    path = "/mundane/pexels-jplenio-2080960.jpg"
    mode = "fill"
  '';
in
{
  # In current nixpkgs, `nix-build '<nixpkgs/nixos>' -A vm` exposes `config.system.build.vm`
  # but does *not* auto-import qemu-vm.nix into our module set, so options like
  # `virtualisation.cores` and `virtualisation.sharedDirectories` would be unknown unless we
  # import it explicitly here.
  imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ];

  # Land straight in a root shell on tty1.
  services.getty.autologinUser = "root";
  users.users.root.extraGroups = [ "seat" "video" "input" ];

  # seatd provides the libseat backend the DRM backend opens its session through.
  services.seatd.enable = true;

  # Mesa / DRI drivers for the GBM + EGL render path (software GL in the VM).
  hardware.graphics.enable = true;

  # Tools available in the guest: the `mundane` launcher and a Wayland terminal for
  # Super+Return.
  environment.systemPackages = [ mundane pkgs.foot pkgs.weston ];
  environment.sessionVariables.MUNDANE_TERMINAL = "foot";

  # Share the repository (built on the host) into the guest at /mundane.
  virtualisation.sharedDirectories.mundane = {
    source = toString ../..;
    target = "/mundane";
  };

  # Drop the default config into root's home so the wallpaper is on out of the box. `L+`
  # forces the symlink to be (re)created on each boot — the user can still override by
  # `rm /root/.config/mundane/config.toml` and writing their own.
  systemd.tmpfiles.rules = [
    "d /root/.config 0755 root root - -"
    "d /root/.config/mundane 0755 root root - -"
    "L+ /root/.config/mundane/config.toml - - - - ${mundaneConfig}"
  ];

  # Resources + reliable evdev input devices for libinput.
  virtualisation.memorySize = 2048;
  virtualisation.cores = 4;
  virtualisation.qemu.options = [
    "-device virtio-keyboard-pci"
    "-device virtio-mouse-pci"
  ];

  # Print usage on login.
  programs.bash.loginShellInit = ''
    cat <<'EOF'

  mundaneDE DRM/KMS test VM.

    Run the compositor:   mundane
    More logging:         RUST_LOG=debug mundane

  Keys (Super = left Meta): Super+Return terminal · Super+W reload wallpaper
                            Super+Q close window · Super+Shift+Q or Esc quit

  The bundled wallpaper is on by default (config at ~/.config/mundane/config.toml).
  Delete that file to fall back to the dark clear colour. See tests/qemu/README.md.

EOF
  '';

  system.stateVersion = "24.05";
}
