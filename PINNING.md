# Smithay Version Pin

| Field       | Value                                      |
|-------------|--------------------------------------------|
| Version     | v0.7.0                                     |
| Tag         | `v0.7.0`                                   |
| Commit      | `a166cf4c94b5aedc332a65aa1dd753e8148829c3`  |
| Date pinned | 2025-06-24 (release date)                  |
| Pinned in   | `Cargo.toml` via `tag = "v0.7.0"`          |

## Rationale

v0.7.0 is the latest stable Smithay release as of project inception. It provides all
required features: DRM/GBM backend, Winit backend, `wlr-layer-shell`, `xdg-shell`,
`linux-dmabuf`, GLES renderer, and the `desktop` module (`Space`, `Window`).

## Updating

To update Smithay, change the `tag` field in `Cargo.toml` for both `smithay` and
`smithay-drm-extras`, run `cargo update`, verify the build, and update this file.
