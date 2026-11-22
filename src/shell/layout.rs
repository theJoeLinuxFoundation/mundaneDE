use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as Flag;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::shell::xdg::ToplevelSurface;
use std::cell::Cell;

use crate::state::State;

#[derive(Default)]
struct Layout {
    normal: Cell<Option<Rectangle<i32, Logical>>>,
    expanded: Cell<bool>,
}

impl State {
    pub(super) fn set_expanded(&self, surface: &ToplevelSurface, flag: Flag, enabled: bool) {
        let Some(w) = self.wm.window_for_surface(surface.wl_surface()) else {
            return;
        };
        w.user_data().insert_if_missing(Layout::default);
        let layout = w.user_data().get::<Layout>().unwrap();
        let geometry = self.space.element_geometry(&w);
        surface.with_pending_state(|s| {
            if enabled {
                s.states.set(flag);
            } else {
                s.states.unset(flag);
            }
            if s.states.contains(Flag::Fullscreen) || s.states.contains(Flag::Maximized) {
                if layout.normal.get().is_none() {
                    layout.normal.set(geometry);
                }
                s.size = Some(self.output_logical_size());
            } else {
                s.size = layout.normal.get().map(|g| g.size);
            }
        });
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    pub fn resize_expanded_windows(&self) {
        let size = self.output_logical_size();
        for window in self.space.elements().chain(&self.minimized) {
            if let Some(t) = window.toplevel() {
                t.with_pending_state(|s| {
                    if s.states.contains(Flag::Maximized) || s.states.contains(Flag::Fullscreen) {
                        s.size = Some(size);
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
        }
    }

    pub(super) fn apply_layout(&mut self, w: &Window) {
        let Some(layout) = w.user_data().get::<Layout>() else {
            return;
        };
        let flags = w.toplevel().unwrap().current_state().states;
        let expanded = flags.contains(Flag::Maximized) || flags.contains(Flag::Fullscreen);
        if layout.expanded.replace(expanded) == expanded {
            return;
        }
        let position = if expanded {
            Point::from((0, 0))
        } else {
            layout.normal.take().map(|g| g.loc).unwrap_or_default()
        };
        if let Some(s) = self
            .wm
            .spring_id(w)
            .and_then(|id| self.animation.spring_mut(id))
        {
            s.set_target(position.x, position.y);
            s.x.snap_to_target();
            s.y.snap_to_target();
        }
        if !self.minimized.contains(w) {
            self.space.map_element(w.clone(), position, false);
        }
    }

    pub fn restore_window(&mut self, window: &Window) {
        self.minimized.retain(|w| w != window);
        if let Some(spring) = self
            .wm
            .spring_id(window)
            .and_then(|id| self.animation.spring(id))
        {
            self.space
                .map_element(window.clone(), spring.rounded_position(), true);
        } else {
            self.map_new_window(window.clone());
        }
        crate::focus::set_keyboard_focus(self, Some(window));
    }
}
