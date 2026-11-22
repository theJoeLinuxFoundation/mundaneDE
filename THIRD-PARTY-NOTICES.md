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
and freedesktop-icons are MIT-licensed. For self_cell use its Apache-2.0 option.
The compositor has its own lockfile.

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
