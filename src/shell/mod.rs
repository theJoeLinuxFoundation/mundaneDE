//! xdg-shell handling and window mapping.
//!
//! New toplevels are registered with the [`WindowManager`](crate::wm::WindowManager) but
//! not mapped until they have committed a buffer; at that point the window is placed,
//! given a spring, and animated into view. The commit path also drives the xdg initial
//! configure handshake.

use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Serial, Size};
use smithay::wayland::compositor::{get_parent, is_sync_subsurface, with_states};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::delegate_xdg_shell;

use crate::state::State;

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|s| {
            s.states.set(xdg_toplevel::State::Activated);
        });
        let window = Window::new_wayland_window(surface);
        self.wm.register(window);
        // The initial configure is sent on the first commit (see `handle_commit`).
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}
    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}
    fn reposition_request(&mut self, _surface: PopupSurface, _positioner: PositionerState, _token: u32) {}

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.wm.window_for_surface(surface.wl_surface()) {
            self.space.unmap_elem(&window);
            if let Some(spring) = self.wm.remove(&window) {
                self.animation.remove_spring(spring);
            }
        }
        crate::focus::refocus(self);
    }
}

impl State {
    /// Logical size of the output (scale 1 in the winit MVP).
    pub fn output_logical_size(&self) -> Size<i32, Logical> {
        self.output
            .current_mode()
            .map(|m| m.size.to_logical(1))
            .unwrap_or_else(|| (1920, 1080).into())
    }

    /// Drive buffer management, the initial-configure handshake, and first-buffer mapping.
    pub fn handle_commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        if is_sync_subsurface(surface) {
            return;
        }

        // Resolve the root (toplevel) surface this commit belongs to.
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        let Some(window) = self.wm.window_for_surface(&root) else {
            return;
        };
        window.on_commit();

        // xdg requires the initial configure be sent before the client attaches a buffer.
        if !initial_configure_sent(&root) {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_configure();
            }
            return;
        }

        let has_buffer = with_renderer_surface_state(&root, |s| s.buffer().is_some()).unwrap_or(false);
        if has_buffer && !self.wm.is_mapped(&window) {
            self.map_new_window(window);
        } else if self.wm.is_mapped(&window) {
            let size = window.geometry().size;
            self.wm.set_size(&window, size);
        }
    }

    /// Place a newly buffered window, give it a spring, and animate it into view.
    fn map_new_window(&mut self, window: Window) {
        let size = window.geometry().size;
        let output_size = self.output_logical_size();
        let target = self.wm.initial_placement(size, output_size);

        // Spring in from just below the target for a gentle entrance.
        let params = self.config.spring.to_spring_params();
        let start = (target.x, target.y + 80);
        let spring = self.animation.create_spring(start.0, start.1, params);
        if let Some(handle) = self.animation.spring_mut(spring) {
            handle.set_target(target.x, target.y);
        }

        self.space.map_element(window.clone(), start, true);
        self.wm.add_mapped(window.clone(), spring, size);

        tracing::info!(
            "mapped window {}x{}, springing in to ({}, {})",
            size.w,
            size.h,
            target.x,
            target.y
        );
        crate::focus::set_keyboard_focus(self, Some(&window));
    }
}

/// Whether the xdg toplevel's initial configure has already been sent.
fn initial_configure_sent(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|d| d.lock().unwrap().initial_configure_sent)
            .unwrap_or(false)
    })
}

delegate_xdg_shell!(State);
