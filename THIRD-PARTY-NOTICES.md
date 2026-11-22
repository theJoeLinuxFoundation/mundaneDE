# Third-party notices

## Settings panel

The `mundane-settings` and `mundane-settings-core` crates are derived from the
argentumOS settings panel and retain the MIT licence. The original workspace
credits the argentumOS contributors. See the [MIT licence](settings-panel/LICENSE).

## Smithay

The popup-grab and selection integration follows Smithay’s Anvil reference
compositor. Its MIT copyright and permission notice are included in
[the Smithay licence](licenses/Smithay-MIT.txt).

## Earlier wallpaper

Earlier commits include `pexels-jplenio-2080960.jpg`, a photograph by Johannes
Plenio. The Pexels Licence applies to copies of that photograph in Git history.
The current wallpaper, Afterlight, is an original procedural landscape covered
by the repository's BSD 3-Clause licence. [Original photograph](https://www.pexels.com/photo/2080960/) ·
[Pexels Licence](https://www.pexels.com/license/)

## Dependencies

Cargo fetches dependencies under their respective upstream licences. These
include Smithay (MIT) and GPUI (Apache-2.0). The new `mundane-shell` crate
uses the repository’s BSD-3-Clause licence. It shares the Settings lockfile and
the suite’s `mundane-ui` (BSD-3-Clause AND Apache-2.0). Its GIO Rust bindings
and freedesktop-icons are MIT-licensed. The capture backend uses nix (MIT)
and tempfile (MIT OR Apache-2.0). For self_cell use its Apache-2.0 option.
The authentication agent uses zeroize (Apache-2.0 OR MIT) and dynamically links the system’s libpolkit-agent-1 and libpolkit-gobject-1 (LGPL-2.0-or-later). The session launches the separately installed Orca executable. The compositor has its own lockfile.

The BSD-3-Clause `mundane-lock` crate shares the shell’s GPUI and mundane-ui
dependencies. For Linux-PAM it uses pam-client 0.5.0 (MPL-2.0, default features
disabled) and pam-sys 1.0.0-alpha5 (MIT OR Apache-2.0). Their source and licence
files are included in the vendored archive. Bindgen uses libclang at build time.
The installed locker dynamically links system libpam.so.0 (Linux-PAM’s BSD
licence option). The distribution supplies PAM modules and their notices.

The native `mundane-session` leader is BSD-3-Clause. It uses GIO/GLib (MIT bindings), tempfile and signal-hook (MIT OR Apache-2.0).

The lock clock uses the MIT-licensed glib Rust binding and the system GLib
library. The input-method view exchanges messages using the IBus panel D-Bus
interface. The distribution ships the IBus daemon, conversion engines and
`ibus-wayland` as separate programs with their upstream notices.

To list the packages, versions and declared licences in both locked dependency
graphs, run the following from the repository root with Cargo and jq installed.

```sh
for manifest in Cargo.toml settings-panel/Cargo.toml; do
  cargo metadata --locked --format-version 1 --manifest-path "$manifest" |
    jq -r '.packages[] | [.name, .version, (.license // .license_file // "see upstream")] | @tsv' |
    sort
done
```

A licence expression may list alternatives or multiple obligations. Consult the
licence and notice files in each package for its full terms. Native libraries
provided by the operating system have their own licences and package notices.
