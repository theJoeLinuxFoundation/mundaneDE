//! The magnetic window manager.
//!
//! Window positions are driven by springs (see [`crate::animation`]): the WM sets a
//! spring *target* and the render loop animates the window toward it each frame. When a
//! window is dropped after an interactive move, [`snap::snap`] pulls it flush to the
//! nearest output edge or neighbour within `attraction_radius`, and the spring carries it
//! there — that is the "magnetic" feel.
//!
//! This module owns only the bookkeeping (which window has which spring and size); the
//! snapping decision is the pure, unit-tested [`snap`] submodule.

mod snap;

pub use snap::snap;

use std::collections::HashMap;

use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::animation::SpringId;

/// A window that has been mapped into the [`Space`] and is being animated.
struct ManagedWindow {
    window: Window,
    spring: SpringId,
    size: Size<i32, Logical>,
}

/// Tracks toplevels and the springs that animate their positions.
pub struct WindowManager {
    /// All known toplevels, keyed by their toplevel surface. Populated when the toplevel
    /// is first created, before it has a buffer / is mapped.
    surfaces: HashMap<WlSurface, Window>,
    /// Windows mapped into the [`Space`] (have a buffer and a spring).
    mapped: Vec<ManagedWindow>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            surfaces: HashMap::new(),
            mapped: Vec::new(),
        }
    }

    /// Record a freshly created toplevel so it can be found on its first commit.
    pub fn register(&mut self, window: Window) {
        if let Some(surface) = toplevel_surface(&window) {
            self.surfaces.insert(surface, window);
        }
    }

    /// Look up the window owning a toplevel surface.
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.surfaces.get(surface).cloned()
    }

    pub fn is_mapped(&self, window: &Window) -> bool {
        self.mapped.iter().any(|m| &m.window == window)
    }

    /// Mark a window as mapped, associating it with its spring and current size.
    pub fn add_mapped(&mut self, window: Window, spring: SpringId, size: Size<i32, Logical>) {
        self.mapped.push(ManagedWindow {
            window,
            spring,
            size,
        });
    }

    pub fn spring_id(&self, window: &Window) -> Option<SpringId> {
        self.mapped
            .iter()
            .find(|m| &m.window == window)
            .map(|m| m.spring)
    }

    pub fn size_of(&self, window: &Window) -> Option<Size<i32, Logical>> {
        self.mapped
            .iter()
            .find(|m| &m.window == window)
            .map(|m| m.size)
    }

    /// Update the cached size of a mapped window (geometry can change after map).
    pub fn set_size(&mut self, window: &Window, size: Size<i32, Logical>) {
        if let Some(m) = self.mapped.iter_mut().find(|m| &m.window == window) {
            m.size = size;
        }
    }

    /// Remove a window from all tracking, returning its spring id (if it was mapped) so
    /// the caller can release the spring.
    pub fn remove(&mut self, window: &Window) -> Option<SpringId> {
        self.surfaces.retain(|_, w| w != window);
        if let Some(pos) = self.mapped.iter().position(|m| &m.window == window) {
            Some(self.mapped.remove(pos).spring)
        } else {
            None
        }
    }

    /// Current space-geometry of every mapped window except `window` — the neighbours a
    /// dragged window can snap against.
    pub fn neighbors_except(
        &self,
        window: &Window,
        space: &Space<Window>,
    ) -> Vec<Rectangle<i32, Logical>> {
        self.mapped
            .iter()
            .filter(|m| &m.window != window)
            .filter_map(|m| space.element_geometry(&m.window))
            .collect()
    }

    /// Cascade new windows around the centre so multiple opens don't fully overlap.
    pub fn initial_placement(
        &self,
        size: Size<i32, Logical>,
        output: Size<i32, Logical>,
    ) -> Point<i32, Logical> {
        let base_x = (output.w - size.w).max(0) / 2;
        let base_y = (output.h - size.h).max(0) / 2;
        let cascade = 32 * (self.mapped.len() as i32 % 6);
        Point::from(((base_x + cascade).max(0), (base_y + cascade).max(0)))
    }

    /// Iterate mapped windows with their spring ids (for syncing positions each frame).
    pub fn mapped_windows(&self) -> impl Iterator<Item = (Window, SpringId)> + '_ {
        self.mapped.iter().map(|m| (m.window.clone(), m.spring))
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

/// The toplevel surface backing a window, if it is an xdg toplevel.
fn toplevel_surface(window: &Window) -> Option<WlSurface> {
    window.toplevel().map(|t| t.wl_surface().clone())
}
