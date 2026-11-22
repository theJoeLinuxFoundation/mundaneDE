# mundane-settings

The native mundaneDE system settings panel — a Rust / [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) application. It tunes the compositor (spring physics, magnetic gaps, wallpaper) and exposes the per-system pages a desktop expects (display, network, sound, users, software, date/time, system).

## Architecture

```
settings-panel/
├── mundane-settings/          # binary crate (UI)
│   └── src/
│       ├── main.rs            # CLI args, window opens here
│       ├── app.rs             # root layout + optimistic helper
│       ├── sidebar.rs         # sidebar (selection on the row itself)
│       ├── theme.rs           # portal subscription and semantic palette
│       └── pages/             # one file per category
│           ├── mod.rs         # Page enum, PageState<T> cache, router
│           ├── compositor.rs  # mundaneDE-native: spring, magnetism, wallpaper mode
│           ├── appearance.rs  # GTK theme + icon theme (gsettings) + wallpaper (mundaneDE TOML)
│           ├── display.rs
│           ├── sound.rs
│           ├── network.rs
│           ├── users.rs
│           ├── software.rs
│           ├── datetime.rs
│           └── system.rs
└── mundane-settings-core/     # library crate (backends; ZERO GPUI imports)
    └── src/
        ├── lib.rs             # public API, error type
        ├── compositor.rs      # ~/.config/mundane/config.toml round-trip (toml_edit)
        ├── display.rs         # mundaneDE output transactions and X11 queries
        ├── theme_scan.rs      # GTK theme paths + gsettings
        ├── wallpaper_scan.rs  # /etc/backgrounds + ~/Pictures
        ├── os_release.rs      # parse /etc/os-release
        ├── sound.rs           # pactl wrapper
        ├── datetime.rs        # timedatectl wrapper
        ├── system.rs          # Packages on joe and NixOS upgrade support
        └── dbus/
            ├── network.rs     # NetworkManager (zbus async)
            ├── accounts.rs    # AccountsService (zbus async)
            └── flatpak.rs     # `flatpak` CLI wrapper
```

**The two crates are split deliberately.** `mundane-settings-core` has no UI framework deps and could be reused by a CLI, a CI test rig, or a future native Wayland panel. The binary crate is the only place GPUI types appear.

## How settings reach mundaneDE

The Compositor page edits `~/.config/mundane/config.toml` via [`toml_edit`](https://docs.rs/toml_edit) (preserves comments). mundaneDE installs an inotify watcher on that path and applies the diff live — spring retune, gap update, wallpaper crossfade — without a restart. The Display page lists modes and tests layouts over the private compositor socket. A trial reverts after 15 seconds unless confirmed.

## UI philosophy

These rules are load-bearing — when in doubt during implementation, follow the rule even if it's harder to code:

1. **Direct manipulation, no Save buttons.** A change is the commit. Exceptions: destructive/expensive operations (display res, system update) use an explicit Apply.
2. **No window chrome.** No titlebar, no menubar. Sidebar + content fill the window.
3. **Calm motion.** Match mundaneDE itself — the compositor animates windows with spring physics; the panel animates nothing.
4. **Hierarchy through space + color.** Four backgrounds (`bg → sidebar → surface → accent`).
5. **Latency honesty.** Instant (<50ms, no indicator) / Optimistic (UI flips, accent underline visible 1s min) / Confirmed (Apply + inline spinner).
6. **Errors inline, never modal.**
7. **Keyboard-reachable.** Focus rings = accent at 50% alpha.
8. **Skeletons, not spinners.** Layout is drawn on first paint with placeholder rows.
9. **Density.** Apple System Settings, not Windows Control Panel. 16/24px padding.
10. **One source of truth.** Compositor settings live in `config.toml`; per-system settings live in dconf/gsettings/D-Bus. The UI is a view over system state.

## State, caching, optimistic updates

Each page owns a `PageState<T>`: `Empty | Loading | Loaded { data, fetched_at } | Error`. Stale-while-revalidate: cached data is shown immediately on page revisit; a background refresh runs if `fetched_at` exceeds the page TTL.

**Optimistic write protocol** (centralized in `app::optimistic_write`):

1. Snapshot the current value.
2. Apply locally, re-render.
3. Mark control in-flight (accent underline, 1s minimum visible).
4. `cx.spawn` the async write.
5. On success: clear in-flight marker.
6. On failure: revert via snapshot, clear marker, show inline error chip with Retry (auto-dismisses after 6s).

## Color palette

The Appearance page writes desktop GSettings preferences. Settings reads them
through the Settings portal and refreshes its GPUI windows on change. Focus,
selection, borders and text use the pinned mundane-ui semantic palette. GTK
clients use native theme settings. Super+Alt+S enables the session reader.

## Running in dev

```bash
cd settings-panel
# Optional with Nix: enter nix-shell here for native dependencies.
cargo run --locked -p mundane-settings -- --page compositor
```

`--page <name>` deep-links to a specific page on launch. Valid values: `compositor`, `appearance`, `display`, `sound`, `network`, `users`, `software`, `date-time`, `system`. The panel is also launched by mundaneDE on `Super+,` (override the binary with `$MUNDANE_SETTINGS`).

## Building with Nix

From the repository root:

```bash
nix-build -E 'with import <nixpkgs> {}; callPackage ./settings-panel {}'
```

## Bumping the GPUI pin

Settings pins Zed `f4fe0be1f8c7974db07e01c5cd05bee0cf4eb8d9`, the upstream
AccessKit revision selected with mundane-apps and gpui-shades. Settings starts through `mundane_ui::run`, which installs the live theme and enables the Linux bridge. The headless screen-reader checks are part of [milestone 7](../docs/plan.md#7-share-settings-appearance-and-accessibility).

1. Set both `gpui` and `gpui_platform` in the workspace manifest to the agreed full revision. Preserve their platform features.
2. From `settings-panel/`, refresh and review `Cargo.lock`. The AccessKit bump needed a workspace `cargo update` to resolve the futures and WebAssembly dependencies together.
3. Run `cargo metadata --locked --format-version 1`. Check that each GPUI package name resolves to one package ID and that every GPUI Git source ends in the selected commit.
4. Run `cargo build --locked --workspace --all-targets` and `cargo test --locked --workspace` with private test state.
5. Update one `cargoLock.outputHashes` entry per distinct locked Git revision in `default.nix`. For a changed source set its hash to `lib.fakeHash`, run the Nix build above and replace the placeholder with the reported recursive hash. Repeat until the build passes and commit the manifest, lockfile and hashes together.

## How to add a new settings page

1. Add a variant to `Page` in `mundane-settings/src/pages/mod.rs`. Pick a TTL via the `ttl()` match arm.
2. Add a label + icon in `Page::label()` / `Page::icon()`.
3. Create `src/pages/foo.rs` mirroring an existing page (start from `users.rs` — it's the simplest). Implement `Render`, own a `PageState<T>`, and call `spawn_refresh` on `new`.
4. Wire it into `PagesView` in `src/pages/mod.rs` (constructor + match arm in `render`).

## Known stubs / TODOs

- **Inline GPUI text input** — text capture is currently routed through `widgets::prompt` (a `zenity` modal subprocess). When GPUI gains a stable inline text-input primitive, `widgets::text_input` becomes the in-pane drop-in replacement.
- **Bluetooth page** — deferred. No upstream tray applet on mundaneDE yet.

## Tests

```bash
cargo test -p mundane-settings-core
```

Covers the pure parsers (xrandr output, `/etc/os-release`, `flatpak remotes` columns) and the `compositor` TOML round-trip. The async D-Bus paths aren't tested here.
