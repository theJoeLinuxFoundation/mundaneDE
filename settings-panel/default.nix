{ lib
, rustPlatform
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
      "collections-0.1.0" = "sha256-480HTo6kHhJ1bVUzHBxYhWI6gVCpoVyPuNNMPQ6wWbw=";
      "naga-29.0.3" = "sha256-jwPdrd2XLvK5ddEutR/39OLMh2JU3UXNWIcJKCndh+U=";
      "xim-ctext-0.3.0" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
      "zed-scap-0.0.8-zed" = "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
      "zed-font-kit-0.14.1-zed" = "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
    };
  };

  env = { RUST_MIN_STACK = "16777216"; };

  nativeBuildInputs = [ pkg-config makeWrapper ];

  buildInputs = [
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
