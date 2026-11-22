//! Resize the geometry around the stationary edge and respect xdg size limits.
use crate::state::State;
use smithay::{
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as Flag},
    utils::{Logical, Point, Rectangle},
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
};

pub struct Resize {
    pub window: Window,
    pub geometry: Rectangle<i32, Logical>,
    pub start: Point<f64, Logical>,
    pub edge: ResizeEdge,
}

impl State {
    pub fn resize_to(&mut self, point: Point<f64, Logical>) {
        let Some(resize) = &self.resize else { return };
        let Some(t) = resize.window.toplevel() else {
            return;
        };
        let delta = (point - resize.start).to_i32_round::<i32>();
        let (min, max) = with_states(t.wl_surface(), |s| {
            let mut attrs = s.cached_state.get::<SurfaceCachedState>();
            let current = attrs.current();
            (current.min_size, current.max_size)
        });
        let edge = resize.edge as u32;
        let mut geo = resize.geometry;
        fn dimension(old: i32, delta: i32, min: i32, max: i32) -> i32 {
            (old + delta).clamp(
                min.max(1),
                if max == 0 {
                    i32::MAX
                } else {
                    max.max(min).max(1)
                },
            )
        }
        if edge & 12 != 0 {
            let left = edge & 4 != 0;
            geo.size.w = dimension(
                geo.size.w,
                if left { -delta.x } else { delta.x },
                min.w,
                max.w,
            );
            if left {
                geo.loc.x += resize.geometry.size.w - geo.size.w;
            }
        }
        if edge & 3 != 0 {
            let top = edge & 1 != 0;
            geo.size.h = dimension(
                geo.size.h,
                if top { -delta.y } else { delta.y },
                min.h,
                max.h,
            );
            if top {
                geo.loc.y += resize.geometry.size.h - geo.size.h;
            }
        }
        t.with_pending_state(|s| {
            s.size = Some(geo.size);
            s.states.set(Flag::Resizing);
        });
        t.send_pending_configure();
        let w = resize.window.clone();
        self.place_window(&w, geo.loc);
    }

    pub fn place_window(&mut self, window: &Window, pos: Point<i32, Logical>) {
        self.space.map_element(window.clone(), pos, false);
        if let Some(s) = self
            .wm
            .spring_id(window)
            .and_then(|id| self.animation.spring_mut(id))
        {
            s.set_target(pos.x, pos.y);
            s.x.snap_to_target();
            s.y.snap_to_target();
        }
        if let Some(x) = window.x11_surface() {
            let mut geo = x.geometry();
            geo.loc = pos;
            let _ = x.configure(geo);
        }
    }
}
