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

    /// Place a new window. First tries the output centre; if a candidate point would
    /// land inside an existing mapped window's rectangle, walks a 32 px spiral outward
    /// until it finds a free spot. Falls back to the legacy `32 × (count % 6)` cascade
    /// if every candidate in the spiral is occupied.
    pub fn initial_placement(
        &self,
        size: Size<i32, Logical>,
        output: Size<i32, Logical>,
        space: &Space<Window>,
    ) -> Point<i32, Logical> {
        let base_x = (output.w - size.w).max(0) / 2;
        let base_y = (output.h - size.h).max(0) / 2;
        let occupied: Vec<Rectangle<i32, Logical>> = self
            .mapped
            .iter()
            .filter_map(|m| space.element_geometry(&m.window))
            .collect();

        let try_point = |x: i32, y: i32| {
            // Pre-clamp: candidate must fit on the output. If the window's wider than
            // the output, fall back to the top-left so it at least starts visible.
            if x < 0 || y < 0 || x + size.w > output.w || y + size.h > output.h {
                return None;
            }
            let cand = Rectangle::new(Point::from((x, y)), size);
            let collides = occupied.iter().any(|r| rects_overlap(&cand, r));
            if collides { None } else { Some(Point::from((x, y))) }
        };

        // Centre first; then a 4-ring 32 px spiral.
        if let Some(p) = try_point(base_x, base_y) {
            return p;
        }
        for (dx, dy) in spiral_offsets(32, 4) {
            if let Some(p) = try_point(base_x + dx, base_y + dy) {
                return p;
            }
        }

        // Legacy cascade fallback.
        let cascade = 32 * (self.mapped.len() as i32 % 6);
        Point::from(((base_x + cascade).max(0), (base_y + cascade).max(0)))
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

/// Yield ring offsets in a square spiral around (0, 0): ring 1 has the 8 cells at radius
/// `step`, ring 2 the 16 cells at radius `2·step`, etc.
fn spiral_offsets(step: i32, rings: i32) -> impl Iterator<Item = (i32, i32)> {
    (1..=rings).flat_map(move |r| {
        let n = r * step;
        let mut v = Vec::new();
        // Top + bottom rows of this ring.
        for dx in (-n..=n).step_by(step as usize) {
            v.push((dx, -n));
            v.push((dx, n));
        }
        // Left + right columns (excluding the corners already pushed).
        for dy in ((-n + step)..=(n - step)).step_by(step as usize) {
            v.push((-n, dy));
            v.push((n, dy));
        }
        v.into_iter()
    })
}

/// True if two axis-aligned rectangles share any interior point.
fn rects_overlap(a: &Rectangle<i32, Logical>, b: &Rectangle<i32, Logical>) -> bool {
    let ax2 = a.loc.x + a.size.w;
    let ay2 = a.loc.y + a.size.h;
    let bx2 = b.loc.x + b.size.w;
    let by2 = b.loc.y + b.size.h;
    a.loc.x < bx2 && b.loc.x < ax2 && a.loc.y < by2 && b.loc.y < ay2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spiral_offsets_first_ring_has_eight_cells() {
        let v: Vec<_> = spiral_offsets(32, 1).collect();
        // 3 cols × 3 rows minus the centre = 8 cells around the origin.
        assert_eq!(v.len(), 8);
        assert!(v.contains(&(0, -32)));
        assert!(v.contains(&(-32, 0)));
        assert!(v.contains(&(32, 32)));
    }

    #[test]
    fn rects_overlap_basic() {
        let a = Rectangle::new(Point::from((0, 0)), Size::from((100, 100)));
        let b = Rectangle::new(Point::from((50, 50)), Size::from((100, 100)));
        let c = Rectangle::new(Point::from((200, 0)), Size::from((50, 50)));
        assert!(rects_overlap(&a, &b));
        assert!(!rects_overlap(&a, &c));
    }
}
