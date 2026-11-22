//! xdg-shell handling and window mapping.
//!
//! New toplevels are registered with the [`WindowManager`](crate::wm::WindowManager) but
//! not mapped until they have committed a buffer; at that point the window is placed,
//! given a spring, and animated into view. The commit path also drives the xdg initial
//! configure handshake.

use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
pub(crate) mod interaction;
mod layout;
pub(crate) mod menu;
mod protocols;
pub(crate) mod services;
use smithay::delegate_xdg_shell;
use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, PopupKeyboardGrab, PopupKind,
    PopupPointerGrab, PopupUngrabStrategy, Window,
};
use smithay::input::{pointer::Focus, Seat};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{protocol::wl_output::WlOutput, Resource};
use smithay::utils::{Logical, Serial, Size};
use smithay::wayland::compositor::{get_parent, is_sync_subsurface};
use smithay::wayland::input_method::{InputMethodHandler, InputMethodSeat};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};

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

    fn new_popup(&mut self, surface: PopupSurface, _: PositionerState) {
        self.position_popup(&surface);
        if let Err(e) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!(%e, "cannot attach popup");
        }
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        if Seat::from_resource(&seat).as_ref() != Some(&self.seat)
            || !self.valid_serial(serial, surface.wl_surface().client().map(|c| c.id()))
        {
            surface.send_popup_done();
            return;
        }
        let popup = PopupKind::Xdg(surface.clone());
        let Ok(root) = find_popup_root_surface(&popup) else {
            return;
        };
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if !keyboard
            .current_focus()
            .is_some_and(|s| s.surface.id().same_client_as(&root.id()))
        {
            surface.send_popup_done();
            return;
        }
        if let Ok(mut grab) = self.popups.grab_popup(
            crate::focus::KeyboardFocus::native(root),
            popup,
            &self.seat,
            serial,
        ) {
            let previous = grab.previous_serial().unwrap_or(serial);
            if (pointer.is_grabbed() && !pointer.has_grab(serial) && !pointer.has_grab(previous))
                || (keyboard.is_grabbed()
                    && !self.seat.input_method().keyboard_grabbed()
                    && !keyboard.has_grab(serial)
                    && !keyboard.has_grab(previous))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|s| s.positioner = positioner);
        self.position_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, t: ToplevelSurface, seat: WlSeat, serial: Serial) {
        let Some((window, geo, start)) = self.grab_geometry(&t, seat, serial) else {
            return;
        };
        self.move_state = Some(crate::state::MoveState {
            window,
            offset: start - geo.loc.to_f64(),
            snap_preview: None,
            client_grab: true,
        });
    }
    fn resize_request(
        &mut self,
        t: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edge: xdg_toplevel::ResizeEdge,
    ) {
        let Some((window, geometry, start)) = self.grab_geometry(&t, seat, serial) else {
            return;
        };
        if edge != xdg_toplevel::ResizeEdge::None {
            self.resize = Some(interaction::Resize {
                window,
                geometry,
                edge,
                start,
            });
        }
    }
    fn show_window_menu(
        &mut self,
        t: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        loc: smithay::utils::Point<i32, Logical>,
    ) {
        if !self.seat.owns(&seat)
            || !self.valid_serial(serial, t.wl_surface().client().map(|c| c.id()))
        {
            return;
        }
        let Some(w) = self.wm.window_for_surface(t.wl_surface()) else {
            return;
        };
        if let Some(pos) = self.space.element_location(&w) {
            self.window_menu = menu::WindowMenu::new(t, pos + loc, self.output_logical_size()).ok();
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.set_expanded(&surface, xdg_toplevel::State::Maximized, true);
    }
    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.set_expanded(&surface, xdg_toplevel::State::Maximized, false);
    }
    fn fullscreen_request(&mut self, surface: ToplevelSurface, _: Option<WlOutput>) {
        self.set_expanded(&surface, xdg_toplevel::State::Fullscreen, true);
    }
    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.set_expanded(&surface, xdg_toplevel::State::Fullscreen, false);
    }
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.wm.window_for_surface(surface.wl_surface()) {
            if !self.minimized.contains(&w) {
                w.set_activated(false);
                surface.send_pending_configure();
                self.space.unmap_elem(&w);
                self.minimized.push(w);
                crate::focus::refocus(self);
            }
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.dismiss_menu_for(&surface);
        if self.resize.as_ref().and_then(|r| r.window.toplevel()) == Some(&surface) {
            self.resize = None;
        }
        if self.move_state.as_ref().and_then(|r| r.window.toplevel()) == Some(&surface) {
            self.move_state = None;
        }
        if let Some(window) = self.wm.window_for_surface(surface.wl_surface()) {
            self.minimized.retain(|w| w != &window);
            self.space.unmap_elem(&window);
            if let Some(spring) = self.wm.remove(&window) {
                self.animation.remove_spring(spring);
            }
        }
        crate::focus::refocus(self);
    }
}

impl State {
    fn grab_geometry(
        &self,
        t: &ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
    ) -> Option<(
        Window,
        smithay::utils::Rectangle<i32, Logical>,
        smithay::utils::Point<f64, Logical>,
    )> {
        let pointer = self.seat.get_pointer()?;
        if !self.seat.owns(&seat) || !pointer.has_grab(serial) {
            return None;
        }
        let start = pointer.grab_start_data()?;
        if start.button != 0x110 || !start.focus?.0.id().same_client_as(&t.wl_surface().id()) {
            return None;
        }
        let flags = t.current_state().states;
        if flags.contains(xdg_toplevel::State::Maximized)
            || flags.contains(xdg_toplevel::State::Fullscreen)
        {
            return None;
        }
        let window = self.wm.window_for_surface(t.wl_surface())?;
        let geometry = self.space.element_geometry(&window)?;
        Some((window, geometry, start.location))
    }
    fn position_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let Some(window) = self.wm.window_for_surface(&root) else {
            return;
        };
        let Some(geo) = self.space.element_geometry(&window) else {
            return;
        };
        let Some(mut bounds) = self.space.output_geometry(&self.output) else {
            return;
        };
        bounds.loc -= geo.loc + get_popup_toplevel_coords(&kind);
        popup.with_pending_state(|s| s.geometry = s.positioner.get_unconstrained_geometry(bounds));
    }

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

        self.popups.commit(surface);
        if let Some(popup) = self.popups.find_popup(surface) {
            match popup {
                PopupKind::Xdg(p) if !p.is_initial_configure_sent() => {
                    let _ = p.send_configure();
                }
                PopupKind::InputMethod(p) => self.popup_repositioned(p),
                _ => {}
            }
            return;
        }
        if is_sync_subsurface(surface) {
            return;
        }

        // Resolve the root (toplevel) surface this commit belongs to.
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        let xwindow = self
            .space
            .elements()
            .find(|w| w.x11_surface().and_then(|x| x.wl_surface()).as_ref() == Some(&root))
            .cloned();
        if let Some(window) = xwindow {
            window.on_commit();
            if window
                .x11_surface()
                .is_some_and(|x| !x.is_override_redirect())
                && surface == &root
                && with_renderer_surface_state(&root, |s| s.buffer().is_some()).unwrap_or(false)
                && !self.x11_focused_once(&window)
            {
                crate::focus::set_keyboard_focus(self, Some(&window));
            }
            return;
        }
        let Some(window) = self.wm.window_for_surface(&root) else {
            return;
        };
        window.on_commit();

        let has_buffer =
            with_renderer_surface_state(&root, |s| s.buffer().is_some()).unwrap_or(false);
        if !has_buffer && self.wm.is_mapped(&window) {
            self.space.unmap_elem(&window);
            self.minimized.retain(|w| w != &window);
            if let Some(id) = self.wm.remove(&window) {
                self.animation.remove_spring(id);
            }
            // A subsequent initial commit starts a new mapping of this role.
            self.wm.register(Window::new_wayland_window(
                window.toplevel().unwrap().clone(),
            ));
            crate::focus::refocus(self);
            return;
        }
        let toplevel = window.toplevel().unwrap();
        if !toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
            return;
        }
        if has_buffer && !self.wm.is_mapped(&window) && !self.minimized.contains(&window) {
            self.map_new_window(window);
        } else if self.wm.is_mapped(&window) {
            let size = window.geometry().size;
            self.wm.set_size(&window, size);
            self.apply_layout(&window);
        }
    }

    /// Place a newly buffered window, give it a spring, and animate it into view.
    fn map_new_window(&mut self, window: Window) {
        let size = window.geometry().size;
        let output_size = self.output_logical_size();
        let parent = window
            .toplevel()
            .and_then(|t| t.parent())
            .and_then(|p| self.wm.window_for_surface(&p))
            .and_then(|w| self.space.element_geometry(&w));
        let target = parent
            .map(|p| {
                (
                    p.loc.x + (p.size.w - size.w) / 2,
                    p.loc.y + (p.size.h - size.h) / 2,
                )
                    .into()
            })
            .unwrap_or_else(|| self.wm.initial_placement(size, output_size, &self.space));

        // Spring in from just below the target for a gentle entrance.
        let params = self.config.spring.to_spring_params();
        let start = (target.x, target.y + 80);
        let spring = self.animation.create_spring(start.0, start.1, params);
        if let Some(handle) = self.animation.spring_mut(spring) {
            handle.set_target(target.x, target.y);
        }

        self.space.map_element(window.clone(), start, true);
        self.wm.add_mapped(window.clone(), spring, size);
        self.apply_layout(&window);

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

delegate_xdg_shell!(State);
