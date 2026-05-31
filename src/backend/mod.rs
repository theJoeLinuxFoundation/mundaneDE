//! Compositor backends.
//!
//! Two backends are planned: [`winit`] (development, runs in a host window) and
//! [`drm`] (production, DRM/KMS direct rendering). The active backend is chosen by
//! the `--winit` CLI flag in `main.rs`. For the current MVP only `winit` is
//! implemented; `drm` is a stub.

pub mod drm;
pub mod winit;
