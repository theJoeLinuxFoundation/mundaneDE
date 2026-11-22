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

fn append_layers(
    renderer: &mut GlesRenderer,
    output: &smithay::output::Output,
    order: [smithay::wayland::shell::wlr_layer::Layer; 2],
    modal: Option<&smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    elements: &mut Vec<MundaneRenderElement>,
) {
    use smithay::backend::renderer::element::AsRenderElements;
    let layers = smithay::desktop::layer_map_for_output(output);
    let scale = output.current_scale().fractional_scale();
    for level in order {
        for layer in layers.layers_on(level).rev() {
            if Some(layer.wl_surface()) == modal {
                continue;
            }
            if let Some(geometry) = layers.layer_geometry(layer) {
                let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = layer
                    .render_elements(
                        renderer,
                        geometry.loc.to_physical_precise_round(scale),
                        scale.into(),
                        1.0,
                    );
                elements.extend(surfaces.into_iter().map(MundaneRenderElement::from));
            }
        }
    }
}

/// Collect the full front-to-back element list for the output, plus the clear colour shown
/// underneath everything. `draw_cursor` is set by backends with no host pointer (DRM); the
/// winit backend leaves it to the host compositor to avoid a double cursor.
pub fn collect_elements(
    renderer: &mut GlesRenderer,
    state: &mut State,
    output: &smithay::output::Output,
    draw_cursor: bool,
) -> (Vec<MundaneRenderElement>, Color32F) {
    let modal = !state.desktop.lock.active && state.auth_surface().is_some();
    let (mut elements, clear) = collect_scene(renderer, state, output, draw_cursor && !modal);
    if modal {
        use smithay::backend::renderer::element::AsRenderElements;
        let bounds = state.space.output_geometry(output).unwrap();
        let scale = output.current_scale().fractional_scale();
        let mut overlay = Vec::new();
        if draw_cursor {
            overlay.extend(cursor_elements(
                renderer,
                state,
                state.cursor_location - bounds.loc.to_f64(),
                scale,
            ));
        }
        let map = smithay::desktop::layer_map_for_output(output);
        if let Some(layer) = map
            .layers()
            .find(|l| Some(l.wl_surface()) == state.auth_surface())
        {
            if let Some(r) = map.layer_geometry(layer) {
                let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = layer
                    .render_elements(
                        renderer,
                        r.loc.to_physical_precise_round(scale),
                        scale.into(),
                        1.,
                    );
                overlay.extend(surfaces.into_iter().map(MundaneRenderElement::from));
            }
        }
        overlay.push(
            SolidColorRenderElement::new(
                state.desktop.lock.mask.clone(),
                smithay::utils::Rectangle::from_size(bounds.size).to_physical_precise_round(scale),
                CommitCounter::default(),
                Color32F::new(0., 0., 0., 0.52),
                Kind::Unspecified,
            )
            .into(),
        );
        overlay.append(&mut elements);
        return (overlay, clear);
    }
    (elements, clear)
}

fn collect_scene(
    renderer: &mut GlesRenderer,
    state: &mut State,
    output: &smithay::output::Output,
    draw_cursor: bool,
) -> (Vec<MundaneRenderElement>, Color32F) {
    let mut elements: Vec<MundaneRenderElement> = Vec::new();

    let bounds = state.space.output_geometry(output).unwrap();
    let scale = output.current_scale().fractional_scale();
    if state.desktop.lock.active {
        if let Some(lock) = state.lock_surface(output) {
            if draw_cursor
                    && state.desktop.lock.gpui_active()
                    && state.desktop.lock.last_input.elapsed().as_secs() < 30
                    && bounds.to_f64().contains(state.cursor_location)
                {
                    let position = (state.cursor_location - bounds.loc.to_f64()).to_physical(scale);
                    if let Ok(arrow) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        position,
                        &state.desktop.lock.arrow,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        elements.push(MundaneRenderElement::from(arrow));
                    }
                }
                let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    renderer,
                    &lock,
                    (0, 0),
                    scale,
                    1.0,
                    Kind::Unspecified,
                );
            elements.extend(surfaces.into_iter().map(MundaneRenderElement::from));
            if state.desktop.lock.gpui_active() {
                // Only the trusted wallpaper texture may appear between the opaque
                // compositor mask and the admitted, transparent locker surface.
                elements.push(
                    SolidColorRenderElement::new(
                        state.desktop.lock.shade.clone(),
                        smithay::utils::Rectangle::from_size(bounds.size)
                            .to_physical_precise_round(scale),
                        CommitCounter::default(),
                        Color32F::new(0., 0., 0., 0.54),
                        Kind::Unspecified,
                    )
                    .into(),
                );
                elements.extend(
                    state
                        .wallpaper
                        .elements(
                            renderer,
                            &state.animation,
                            (bounds.size.w, bounds.size.h),
                            scale,
                        )
                        .into_iter()
                        .map(MundaneRenderElement::from),
                );
            }
        }
        let rect =
            smithay::utils::Rectangle::from_size(bounds.size).to_physical_precise_round(scale);
        elements.push(
            SolidColorRenderElement::new(
                state.desktop.lock.mask.clone(),
                rect,
                CommitCounter::default(),
                Color32F::new(0.04, 0.07, 0.10, 1.0),
                Kind::Unspecified,
            )
            .into(),
        );
        return (elements, Color32F::BLACK);
    }
    let pointer = state.cursor_location - bounds.loc.to_f64();
    // The cursor draws on top of everything.
    if draw_cursor {
        elements.extend(cursor_elements(renderer, state, pointer, scale));
    }

    if let Some(icon) = &state.dnd_icon {
        let icon_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                icon,
                pointer.to_physical_precise_round(scale),
                scale,
                1.0,
                Kind::Unspecified,
            );
        elements.extend(icon_elements.into_iter().map(MundaneRenderElement::from));
    }
    if let Some(menu) = &state.window_menu {
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (menu.origin - bounds.loc).to_f64().to_physical(scale),
            &menu.buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            elements.push(MundaneRenderElement::from(element));
        }
    }
    // Snap-preview ghost: shown only during an interactive drag when the snap target
    // would move the window. Drawn above windows so the user sees where it'll land.
    if let Some(rect) = state.move_state.as_ref().and_then(|mv| mv.snap_preview) {
        let geo = smithay::utils::Rectangle::new(rect.loc - bounds.loc, rect.size)
            .to_physical_precise_round(scale);
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

    if state.auth_surface().is_some()
        || state.desktop.transition.is_some()
        || state
            .space
            .elements()
            .any(crate::shell::decorations::decorated)
    {
        use smithay::backend::renderer::element::AsRenderElements;
        use smithay::wayland::shell::wlr_layer::Layer;
        append_layers(
            renderer,
            output,
            [Layer::Overlay, Layer::Top],
            state.auth_surface(),
            &mut elements,
        );
        let transition = state.desktop.transition.as_ref();
        let progress = transition
            .and_then(|t| state.animation.spring(t.spring))
            .map_or(1., |s| (s.x.position / 100.).clamp(0., 1.) as f32);
        let incoming = state
            .space
            .elements()
            .rev()
            .filter_map(|w| Some((w, state.space.element_geometry(w)?, progress)));
        let outgoing = transition
            .into_iter()
            .flat_map(|t| t.previous.iter())
            .map(|(w, rect)| (w, *rect, 1. - progress));
        for (w, g, alpha) in incoming.chain(outgoing) {
            let origin = g.loc - bounds.loc;
            let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = w.render_elements(
                renderer,
                (origin - w.geometry().loc).to_physical_precise_round(scale),
                scale.into(),
                alpha,
            );
            elements.extend(surfaces.into_iter().map(MundaneRenderElement::from));
            if crate::shell::decorations::decorated(w) {
                let title = state
                    .desktop
                    .tasks
                    .get(w)
                    .map(|t| t.handle.title())
                    .unwrap_or_default();
                if let Some(buffer) = crate::shell::decorations::draw(w, title, g.size.w) {
                    let loc = origin
                        - smithay::utils::Point::from((0, crate::shell::decorations::HEIGHT));
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        loc.to_f64().to_physical(scale),
                        &buffer,
                        Some(alpha),
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        elements.push(el.into());
                    }
                }
            }
        }
        append_layers(
            renderer,
            output,
            [Layer::Bottom, Layer::Background],
            state.auth_surface(),
            &mut elements,
        );
    } else {
        match space_render_elements::<_, Window, _>(renderer, [&state.space], output, 1.0) {
            Ok(windows) => elements.extend(windows.into_iter().map(MundaneRenderElement::from)),
            Err(err) => tracing::warn!("collecting window elements failed: {err:?}"),
        }
    }

    // Wallpaper sits behind the windows, so it is appended last (front-to-back order).

    let wallpaper = state.wallpaper.elements(
        renderer,
        &state.animation,
        (bounds.size.w, bounds.size.h),
        scale,
    );
    elements.extend(wallpaper.into_iter().map(MundaneRenderElement::from));

    // Background clear colour, visible before the wallpaper fades in or if a custom image fails to load.
    let clear_color = Color32F::new(0.08, 0.08, 0.10, 1.0);
    (elements, clear_color)
}

/// Build the cursor render element(s): a client-provided surface if set, otherwise the
/// built-in arrow, positioned in output-local coordinates.
pub(crate) fn cursor_elements(
    renderer: &mut GlesRenderer,
    state: &State,
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    scale: f64,
) -> Vec<MundaneRenderElement> {
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
            let location = (pos - hotspot.to_f64()).to_physical(scale).to_i32_round();
            // Kind::Unspecified, NOT Kind::Cursor: the cursor-plane direct-scanout path
            // (gated by Kind::Cursor in DrmCompositor) is unreliable on virtio-gpu in QEMU
            // and leaves cursor trails when its fallback fails to damage the old position.
            // Compositing on the primary plane like a regular element damages prev+new
            // correctly. We give up the cursor-plane perf optimisation.
            let surface_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    renderer,
                    surface,
                    location,
                    scale,
                    1.0,
                    Kind::Unspecified,
                );
            surface_elements
                .into_iter()
                .map(MundaneRenderElement::from)
                .collect()
        }
        CursorImageStatus::Named(_) => {
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (pos - state.named_cursor.hotspot.to_f64()).to_physical(scale),
                &state.named_cursor.buffer,
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
    // Space::map_element also raises the window. Preserve the stack when a
    // spring moves underneath another window, including a rootless X11 window.
    let order: Vec<_> = state.space.elements().cloned().collect();
    let mut reordered = false;
    for window in order {
        let position = state
            .wm
            .spring_id(&window)
            .and_then(|id| state.animation.spring(id))
            .map(|s| s.rounded_position().into());
        match position {
            Some(pos) if Some(pos) != state.space.element_location(&window) => {
                state.space.map_element(window, pos, false);
                reordered = true;
            }
            _ if reordered => state.space.raise_element(&window, false),
            _ => {}
        }
    }
}
