# Contributing to mundaneDE

Use stable Rust and the committed lockfiles. The [README](README.md) covers
native build dependencies and running a nested session. Use `RUST_LOG=debug`
to see renderer and input diagnostics. To run a single compositor test, pass
its name to Cargo, for example `cargo test --locked spring_converges_to_target`.

## Compositor architecture

- Window positions and wallpaper opacity use damped springs. `src/animation/`
  integrates each axis with semi-implicit Euler steps, subdivides longer frame
  intervals and snaps an axis to its target when both displacement and speed
  fall below their thresholds. The damping ratio is
  `damping / (2 * sqrt(stiffness * mass))`.
- `src/wm/snap.rs` calculates snap targets from screen and window geometry.
  The window manager uses the chosen position as a spring target on release.
  Pure geometry tests cover edge, gap and centre alignment.
- `src/backend/winit.rs` and `src/backend/drm.rs` supply the output and seat
  identity to a shared `State`. Both use the GLES renderer. The winit backend
  uses the host pointer. DRM draws a client cursor surface or the built-in
  arrow from `src/cursor.rs`.
- `src/render.rs` synchronises spring positions into Smithay's `Space` and
  collects render elements front to back. Wallpaper layers come last.
  `src/wallpaper.rs` decodes the image on the CPU, uploads it to a texture on
  first render and keeps the outgoing layer until the crossfade settles.
- `src/config.rs` defines the TOML defaults. `src/hot_reload.rs` watches
  the parent directory so atomic replacements of the config file are picked
  up. Keep the [configuration example](config.example.toml) in sync when
  changing a setting.

Smithay and smithay-drm-extras use the same git tag. Keep calloop and xkbcommon
compatible with that revision. See the [pinning notes](PINNING.md).

The [settings panel](settings-panel/README.md) has its own workspace and
lockfile. Its core library reads and writes compositor settings through the
same TOML file.
