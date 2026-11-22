use std::time::{Duration, Instant};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_server::{backend::ClientId, protocol::wl_surface::WlSurface, Resource};
use smithay::wayland::{shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
    selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
    xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData}};
use smithay::utils::Serial;
use crate::state::State;

impl State {
    pub(crate) fn record_input(&mut self, serial: Serial, focus: Option<WlSurface>) {
        if let Some(client) = focus.and_then(|s| s.client()) {
            if self.recent_input.len() == 32 {
                self.recent_input.pop_front();
            }
            self.recent_input
                .push_back((serial, client.id(), Instant::now()));
        }
    }
    pub(super) fn valid_serial(&self, serial: Serial, client: Option<ClientId>) -> bool {
        self.recent_input.iter().any(|(s, c, t)| {
            *s == serial && Some(c) == client.as_ref() && t.elapsed() < Duration::from_secs(5)
        })
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, surface: ToplevelSurface) {
        self.unset_mode(surface);
    }
    fn request_mode(&mut self, surface: ToplevelSurface, mode: Mode) {
        surface.with_pending_state(|s| s.decoration_mode = Some(mode));
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }
    fn unset_mode(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|s| s.decoration_mode = Some(Mode::ClientSide));
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }
}
impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary
    }
}
impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation
    }
    fn token_created(&mut self, _: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        self.activation
            .retain_tokens(|_, data| data.timestamp.elapsed() < Duration::from_secs(5));
        data.serial
            .is_some_and(|(s, seat)| self.seat.owns(&seat) && self.valid_serial(s, data.client_id))
    }
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.activation.remove_token(&token);
        if data.timestamp.elapsed() < Duration::from_secs(5) {
            if let Some(w) = self.wm.window_for_surface(&surface) {
                if self.minimized.contains(&w) {
                    self.restore_window(&w);
                } else if self.space.element_location(&w).is_some() {
                    crate::focus::set_keyboard_focus(self, Some(&w));
                }
            }
        }
    }
}
smithay::delegate_xdg_activation!(State);
smithay::delegate_xdg_decoration!(State);
smithay::delegate_primary_selection!(State);

impl smithay::wayland::tablet_manager::TabletSeatHandler for State {}
smithay::delegate_cursor_shape!(State);
