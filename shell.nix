{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    clang
    llvmPackages.libclang
    rustup
  ];

  buildInputs = with pkgs; [
    # Wayland core
    wayland
    wayland-protocols
    wayland-scanner

    # Input & keyboard
    libxkbcommon
    libinput

    # Graphics / DRM
    libdrm
    cairo
    mesa
    libgbm            # GBM buffer allocator (split out of mesa in recent nixpkgs)
    libGL
    libdisplay-info   # EDID/DisplayID parsing (smithay DRM extras)

    # Device management
    systemd          # libudev
    seatd            # libseat

    # QEMU for DRM backend testing
    qemu_kvm
  ];

  shellHook = ''
    export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
    export XKB_CONFIG_ROOT="${pkgs.xkeyboard_config}/share/X11/xkb"
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [
      pkgs.wayland
      pkgs.libxkbcommon
      pkgs.libdrm
      pkgs.mesa
      pkgs.libgbm
      pkgs.libinput
      pkgs.systemd
      pkgs.seatd
      pkgs.libGL
      pkgs.libdisplay-info
    ]}:$LD_LIBRARY_PATH"
  '';
}
