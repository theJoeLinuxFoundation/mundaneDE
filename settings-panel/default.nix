{ lib
, rustPlatform
, pam
, glib
, polkit
, pkg-config
, vulkan-loader
, libxkbcommon
, libGL
, wayland
, fontconfig
, freetype
, xorg
, dbus
, libxml2
, openssl
, alsa-lib
, makeWrapper
}:

# mundane-settings — the native mundaneDE system settings panel.
#
# Keep the git dependency hashes in sync with Cargo.lock when updating GPUI.

rustPlatform.buildRustPackage {
  pname = "mundane-settings";
  version = "0.1.0";

  src = ./.;

  cargoLock = {
    lockFile = ./Cargo.lock;
    outputHashes = {
      # One entry per unique git source — the hash is shared across every
      # crate vendored from this Zed rev (gpui, collections, util, …).
      "mundane-ui-0.1.0" = "sha256-dgQZJFb6vyoxDALuohJx6oY6JRL5J97YWqyiNQfmKCg=";
      "collections-0.1.0" = "sha256-1oqvlyK/jrihesfPKCPuj9UFzwbfJlFcA6bi1ewFxqg=";
      "naga-30.0.0" = "sha256-OqPeK1cr3OSZgIl+F3GMuXWz31PxFoOO0u4INN9GmgM=";
      "oo7-0.6.0" = "sha256-QXblWsnq+graWOSW0iA8pUzpwizDLi/kOSWDdSQVZ/0=";
      "wasm_thread-0.3.3" = "sha256-+lRLCIk0S6Y5ORYjDKsYYHia2FtoSoh+rWkQh7mnPBE=";
      "xim-ctext-0.3.0" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
      "zed-scap-0.0.8-zed" = "sha256-WVcjceVESUhPKL9z/1y+l/vmtHRJS4EecaMOaVN686o=";
      "zed-font-kit-0.14.1-zed" = "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
    };
  };

  env = { RUST_MIN_STACK = "16777216"; };

  nativeBuildInputs = [ pkg-config makeWrapper rustPlatform.bindgenHook ];

  cargoBuildFlags = [ "-p" "mundane-settings" ];

  buildInputs = [
    glib
    polkit
    pam
    vulkan-loader
    libxkbcommon
    libGL
    wayland
    fontconfig
    freetype
    dbus
    libxml2
    openssl
    alsa-lib
    xorg.libxcb
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];

  # GPUI requires several libraries to be resolvable at runtime via dlopen.
  # Adding them to rpath avoids "vulkan-loader not found" failures on launch.
  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath [
      vulkan-loader
      libxkbcommon
      libGL
      wayland
      fontconfig
    ]} $out/bin/mundane-settings
  '';

  meta = {
    description = "Native mundaneDE system settings panel (Rust + GPUI)";
    homepage = "https://github.com/theJoeLinuxFoundation/mundaneDE";
    license = lib.licenses.mit;
    mainProgram = "mundane-settings";
    platforms = lib.platforms.linux;
  };
}
