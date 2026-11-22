use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as Flag;
use smithay::utils::{Logical, Rectangle};
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
        let size = self.work_area(&w).size;
        let default_full = self.window_bounds(&w).size;
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
                s.size = Some(if s.states.contains(Flag::Fullscreen) {
                    s.fullscreen_output
                        .as_ref()
                        .and_then(smithay::output::Output::from_resource)
                        .and_then(|o| self.space.output_geometry(&o))
                        .map(|g| g.size)
                        .unwrap_or(default_full)
                } else {
                    size
                });
            } else {
                s.size = layout.normal.get().map(|g| g.size);
            }
        });
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    pub(crate) fn rehome_expanded(
        &self,
        w: &Window,
        delta: smithay::utils::Point<i32, Logical>,
        bounds: Rectangle<i32, Logical>,
    ) -> bool {
        let Some(layout) = w.user_data().get::<Layout>() else {
            return false;
        };
        if let Some(mut saved) = layout.normal.get() {
            saved.loc = crate::outputs::clamp_window(saved.loc + delta, saved.size, bounds);
            layout.normal.set(Some(saved));
        }
        layout.expanded.get()
    }

    pub fn resize_expanded_windows(&self) {
        for window in self.space.elements().chain(&self.minimized) {
            if let Some(t) = window.toplevel() {
                let size = self.work_area(window).size;
                let full = self.window_bounds(window).size;
                t.with_pending_state(|s| {
                    if s.states.contains(Flag::Maximized) || s.states.contains(Flag::Fullscreen) {
                        s.size = Some(if s.states.contains(Flag::Fullscreen) {
                            full
                        } else {
                            size
                        });
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
        let full_bounds = w
            .toplevel()
            .unwrap()
            .current_state()
            .fullscreen_output
            .as_ref()
            .and_then(smithay::output::Output::from_resource)
            .and_then(|o| self.space.output_geometry(&o));
        let relocating = flags.contains(Flag::Fullscreen)
            && full_bounds.is_some_and(|g| self.space.element_location(w) != Some(g.loc));
        if layout.expanded.replace(expanded) == expanded && !relocating {
            return;
        }
        let position = if expanded {
            if flags.contains(Flag::Fullscreen) {
                full_bounds.unwrap_or_else(|| self.window_bounds(w)).loc
            } else {
                self.work_area(w).loc
            }
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
        if !self.desktop.showing_desktop
            && !self.minimized.contains(w)
            && self
                .desktop
                .tasks
                .get(w)
                .is_none_or(|t| t.workspace == self.desktop.workspace)
        {
            self.space.map_element(w.clone(), position, false);
        }
    }

    pub fn restore_window(&mut self, window: &Window) {
        self.leave_desktop();
        self.minimized.retain(|w| w != window);
        if let Some(spring) = self
            .wm
            .spring_id(window)
            .and_then(|id| self.animation.spring(id))
        {
            self.space
                .map_element(window.clone(), spring.rounded_position(), true);
        } else if let Some(x) = window.x11_surface() {
            self.space
                .map_element(window.clone(), x.geometry().loc, true);
        } else {
            self.map_new_window(window.clone());
        }
        crate::focus::set_keyboard_focus(self, Some(window));
        self.publish_tasks();
    }
}
