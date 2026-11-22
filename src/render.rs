//! Shared render-element collection for both backends.
//!
//! The wallpaper must sit *behind* every window, but Smithay's
//! [`desktop::space::render_output`](smithay::desktop::space::render_output) only appends
//! custom elements *on top* of the space. So instead both backends collect one ordered
//! element list here — front-to-back, i.e. windows first and the wallpaper last — and feed
//! it to their own renderer path: the winit backend's [`OutputDamageTracker`], or the DRM
//! backend's `DrmCompositor` (which owns its own damage tracker). Fixing the element enum to
//! the [`GlesRenderer`] both backends use keeps it free of renderer generics.
//!
//! [`OutputDamageTracker`]: smithay::backend::renderer::damage::OutputDamageTracker

use std::sync::Mutex;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::Color32F;
use smithay::desktop::space::{space_render_elements, SpaceRenderElements};
use smithay::desktop::Window;
use smithay::input::pointer::{CursorImageAttributes, CursorImageStatus};
use smithay::wayland::compositor::with_states;

use crate::animation::SpringId;
use crate::state::State;

render_elements! {
    /// Everything mundaneDE draws, fixed to the [`GlesRenderer`] both backends use.
    pub MundaneRenderElement<=GlesRenderer>;
    /// The built-in fallback cursor.
    Cursor = MemoryRenderBufferRenderElement<GlesRenderer>,
    /// A client-provided cursor surface.
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    /// Toplevel (and layer-shell) surfaces collected from the [`Space`].
    Space = SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    /// A wallpaper layer (one per crossfade slot, tiled into several for tile mode).
    Wallpaper = TextureRenderElement<GlesTexture>,
    /// Magnetic snap preview ghost (drawn during a drag when a candidate is in range).
    SnapGhost = SolidColorRenderElement,
}

/// Collect the full front-to-back element list for the output, plus the clear colour shown
/// underneath everything. `draw_cursor` is set by backends with no host pointer (DRM); the
/// winit backend leaves it to the host compositor to avoid a double cursor.
pub fn collect_elements(
    renderer: &mut GlesRenderer,
    state: &mut State,
    draw_cursor: bool,
) -> (Vec<MundaneRenderElement>, Color32F) {
    let mut elements: Vec<MundaneRenderElement> = Vec::new();

    // The cursor draws on top of everything.
    if draw_cursor {
        elements.extend(cursor_elements(renderer, state));
    }

    // Snap-preview ghost: shown only during an interactive drag when the snap target
    // would move the window. Drawn above windows so the user sees where it'll land.
    // Output scale is 1, so logical and physical coordinates coincide and we can
    // retype the rectangle directly without a scale-aware conversion.
    if let Some(rect) = state.move_state.as_ref().and_then(|mv| mv.snap_preview) {
        let loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((rect.loc.x, rect.loc.y));
        let size = smithay::utils::Size::<i32, smithay::utils::Physical>::from((rect.size.w, rect.size.h));
        let geo = smithay::utils::Rectangle::new(loc, size);
        // Fresh Id each frame matches the cursor pattern in this module — the
        // damage tracker damages both the previous frame's ghost (gone) and this
        // frame's ghost (new), which keeps the preview crisp under any motion.
        let ghost = SolidColorRenderElement::new(
            Id::new(),
            geo,
            CommitCounter::default(),
            // Spring-blue at 12% alpha, matching the settings panel ACCENT.
            Color32F::new(0.43, 0.66, 0.99, 0.12),
            Kind::Unspecified,
        );
        elements.push(MundaneRenderElement::from(ghost));
    }

    match space_render_elements::<_, Window, _>(renderer, [&state.space], &state.output, 1.0) {
        Ok(windows) => elements.extend(windows.into_iter().map(MundaneRenderElement::from)),
        Err(err) => tracing::warn!("collecting window elements failed: {err:?}"),
    }

    // Wallpaper sits behind the windows, so it is appended last (front-to-back order).
    let output = state.output_logical_size();
    let wallpaper = state
        .wallpaper
        .elements(renderer, &state.animation, (output.w, output.h));
    elements.extend(wallpaper.into_iter().map(MundaneRenderElement::from));

    // Background clear colour, visible before the wallpaper fades in or if a custom image fails to load.
    let clear_color = Color32F::new(0.08, 0.08, 0.10, 1.0);
    (elements, clear_color)
}

/// Build the cursor render element(s): a client-provided surface if set, otherwise the
/// built-in arrow. Output scale is 1, so logical and physical coordinates coincide.
fn cursor_elements(renderer: &mut GlesRenderer, state: &State) -> Vec<MundaneRenderElement> {
    let pos = state.cursor_location;

    match &state.cursor_status {
        CursorImageStatus::Hidden => Vec::new(),
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .map(|attrs| attrs.lock().unwrap().hotspot)
                    .unwrap_or_default()
            });
            let location = (pos - hotspot.to_f64()).to_physical(1.0).to_i32_round();
            // Kind::Unspecified, NOT Kind::Cursor: the cursor-plane direct-scanout path
            // (gated by Kind::Cursor in DrmCompositor) is unreliable on virtio-gpu in QEMU
            // and leaves cursor trails when its fallback fails to damage the old position.
            // Compositing on the primary plane like a regular element damages prev+new
            // correctly. We give up the cursor-plane perf optimisation.
            let surface_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(renderer, surface, location, 1.0, 1.0, Kind::Unspecified);
            surface_elements
                .into_iter()
                .map(MundaneRenderElement::from)
                .collect()
        }
        // Named / default cursor → the built-in arrow (hotspot at its top-left tip).
        //
        // We rebuild the `MemoryRenderBuffer` every frame (it's a 14×21 RGBA buffer — about
        // 1 KB to re-upload). That gives the element a fresh `Id` every frame, so the
        // [`OutputDamageTracker`] treats the previous frame's cursor as *gone* (damaging its
        // last geometry) and this frame's cursor as *new* (damaging its current geometry).
        // The result is reliable damage at both the old and new positions — no trails.
        //
        // [`OutputDamageTracker`]: smithay::backend::renderer::damage::OutputDamageTracker
        _ => {
            let buffer = crate::cursor::build_cursor();
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                pos.to_physical(1.0),
                &buffer,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                Ok(element) => vec![MundaneRenderElement::from(element)],
                Err(e) => {
                    tracing::warn!("cursor render failed: {e:?}");
                    Vec::new()
                }
            }
        }
    }
}

/// Copy each spring's current position into its window's location in the [`Space`].
pub fn sync_spring_positions(state: &mut State) {
    let pairs: Vec<(Window, SpringId)> = state.wm.mapped_windows().collect();
    for (window, id) in pairs {
        if let Some((x, y)) = state.animation.spring(id).map(|s| s.rounded_position()) {
            state.space.map_element(window, (x, y), false);
        }
    }
}
