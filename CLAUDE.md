# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

mundaneDE ("one of everything") is an early-stage Wayland compositor / desktop environment in Rust, built on the [Smithay](https://github.com/Smithay/smithay) framework (pinned to v0.7.0). Its defining idea: **every animated value — window position, opacity, scale — is driven by a damped-spring physics engine**, paired with a "magnetic" window manager where windows snap toward attraction points.

## Current state

The project **builds and runs**. Both backends are implemented: the `winit` development backend (host window) and the `drm` DRM/KMS production backend (single output). Implemented modules: `animation`, `config`, `state`, `shell`, `wm`, `focus`, `render`, `wallpaper`, `cursor`, and `backend::{winit, drm}`. The magnetic window manager (spring-driven snap), the spring engine, and the wallpaper engine are all wired into a shared render path. `cargo test` covers the spring physics, snap geometry, wallpaper scaling math, and config parsing. A documented `config.example.toml` at the repo root shows every setting.

Backend-specific runtime testing: the winit backend runs in any Wayland/X session; the DRM backend needs a real seat/TTY/GPU and is exercised via the QEMU VM in `tests/qemu/` (see its README).

## Build & run

System libraries (Wayland, DRM/Mesa, libinput, libseat, libxkbcommon, libclang) come from the Nix dev shell — always work inside it:

```bash
nix-shell                 # enters shell.nix; sets LIBCLANG_PATH, XKB_CONFIG_ROOT, LD_LIBRARY_PATH
```

rustup is installed but has no default toolchain set (project targets Rust 1.80.1):

```bash
rustup default 1.80.1     # or `stable`
```

Then, inside `nix-shell`:

```bash
cargo build
cargo run -- --winit      # everyday dev entry point: runs in a host window (Winit backend)
cargo run                 # production path: DRM/KMS direct rendering (needs a free TTY + seat)
cargo test
cargo test spring_converges_to_target   # run a single test by name
```

The DRM/KMS backend needs a real seat/TTY; QEMU is bundled in the dev shell for this. To test it, run `tests/qemu/run.sh` — it builds the compositor and boots a NixOS VM (DRM device + seatd) where you run `mundane`. See `tests/qemu/README.md`. Control log verbosity with `RUST_LOG` (defaults to `info`), e.g. `RUST_LOG=debug cargo run -- --winit`.

Note on toolchain: the pinned Rust 1.80.1 can no longer resolve the current dependency graph (some transitive crates now require Cargo's `edition2024`). Build with `stable` instead (CLAUDE.md permits it). There is a committed `Cargo.lock`.

## Architecture

- **Animation engine (`src/animation/`)** — the heart of the compositor. `SpringState` integrates a damped harmonic oscillator with semi-implicit (symplectic) Euler, stable at 60 Hz. `SpringHandle` pairs two springs into a 2D animated position. `AnimationEngine` owns all springs by `SpringId`; `tick(dt)` advances them each frame and returns whether any are still moving (used to decide whether to schedule another frame). `dt` is clamped to `MAX_DT` (~33 ms) to survive dropped frames; springs settle within `SETTLE_EPSILON` (0.5 px). Tune via the **damping ratio** `zeta = damping / (2*sqrt(stiffness*mass))` — defaults give zeta=0.5 (springy with mild overshoot).
- **Magnetic window manager (`src/wm/`)** — window positions become spring *targets*. Config exposes `attraction_radius` and `gap_size` for snap-to-neighbor/edge behavior; the pure snap math lives in `src/wm/snap.rs` (unit-tested).
- **Backends (`src/backend/`)** — two interchangeable backends chosen at startup by the `--winit` CLI flag in `main.rs`: `winit` (development, host window, absolute pointer) and `drm` (production, DRM/KMS, single output, libinput relative pointer). Each owns its renderer/display externally and passes `&mut State` as the calloop loop data; `State::new` is backend-agnostic and takes a `BackendIdentity` (seat name, output name, mode, transform, physical props) so each backend supplies its own device identity. `State::handle_input<B: InputBackend>` is shared by both.
- **Shared render path (`src/render.rs`)** — `MundaneRenderElement` is a `render_elements!` enum fixed to the `GlesRenderer` both backends use. `collect_elements(renderer, state)` gathers window surfaces from the `Space` (via `space_render_elements`) and appends wallpaper elements *behind* them (front-to-back order), returning the list + clear colour. The winit backend feeds it to `OutputDamageTracker::render_output`; the DRM backend feeds it to `DrmCompositor::render_frame` (which owns its own damage tracker). `sync_spring_positions` (shared) copies each spring's position into the `Space`.
- **Wallpaper engine (`src/wallpaper.rs`)** — decodes a PNG/JPEG to RGBA on the CPU, lazily uploads it to a `GlesTexture` on first render, and renders it behind all windows. Opacity is spring-driven: it fades in on startup and crossfades on change (two layers, incoming on top). Scaling modes: `fill`/`cover`, `fit`/`contain`, `stretch`, `center`, `tile` (geometry in `placements()`, unit-tested). `Super+W` reloads/crossfades the configured wallpaper.
- **Cursor (`src/cursor.rs`)** — `collect_elements`' `draw_cursor` flag is `true` for DRM (no host pointer) and `false` for winit (host draws it). When set, the cursor renders on top: a client-provided cursor surface if one is set, otherwise a built-in procedurally-drawn arrow (`build_cursor()` → `MemoryRenderBuffer`). `State` tracks `cursor_status` (updated by `SeatHandler::cursor_image`) and holds the fallback `cursor_buffer`.
- **Config (`src/config.rs`)** — loads `$XDG_CONFIG_HOME/mundane/config.toml` (falling back to `~/.config/mundane/config.toml`) with `[spring]`, `[wm]`, `[output]`, and `[wallpaper]` sections. Parsing is non-fatal: a missing or malformed file logs a warning and uses defaults (`#[serde(default)]` everywhere). `SpringSection::to_spring_params()` and `WallpaperSection::to_fade_params()` bridge config into the animation engine. `[wallpaper]` keys: `path` (optional image), `mode` (scaling), and `fade_{stiffness,damping,mass}` (fade spring).

## Smithay version pinning

Smithay is pinned to **v0.7.0** (git tag) for both `smithay` and `smithay-drm-extras` in `Cargo.toml`. `calloop` (0.14) and `xkbcommon` (0.8) must stay compatible with that Smithay version. Read `PINNING.md` before changing the pin: update the tag in both deps, run `cargo update`, verify the build, then update `PINNING.md`.
