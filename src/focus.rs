use crate::state::State;
use smithay::{
    backend::input::KeyState,
    desktop::{PopupKind, Window},
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        Seat,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial, SERIAL_COUNTER},
    wayland::{seat::WaylandFocus, text_input::TextInputSeat},
    xwayland::X11Surface,
};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardFocus {
    pub surface: WlSurface,
    pub x11: Option<X11Surface>,
}
impl KeyboardFocus {
    pub fn native(surface: WlSurface) -> Self {
        Self { surface, x11: None }
    }
}
impl From<PopupKind> for KeyboardFocus {
    fn from(p: PopupKind) -> Self {
        Self::native(p.wl_surface().clone())
    }
}
impl From<KeyboardFocus> for WlSurface {
    fn from(f: KeyboardFocus) -> Self {
        f.surface
    }
}
impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(&self.surface))
    }
}
impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        self.surface.alive()
    }
}
impl KeyboardTarget<State> for KeyboardFocus {
    fn enter(
        &self,
        seat: &Seat<State>,
        state: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        if let Some(x) = &self.x11 {
            KeyboardTarget::enter(x, seat, state, keys, serial)
        } else {
            KeyboardTarget::enter(&self.surface, seat, state, keys, serial);
            // Smithay has deactivated the old input method on keyboard enter.
            // Protected fields receive the seat keyboard directly.
            if state.desktop.lock.active || state.auth_surface() == Some(&self.surface) {
                seat.text_input().leave();
                seat.text_input().set_focus(None);
            }
        }
    }
    fn leave(&self, seat: &Seat<State>, state: &mut State, serial: Serial) {
        if let Some(x) = &self.x11 {
            KeyboardTarget::leave(x, seat, state, serial)
        } else {
            KeyboardTarget::leave(&self.surface, seat, state, serial)
        }
    }
    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        if let Some(x) = &self.x11 {
            KeyboardTarget::key(x, seat, data, key, state, serial, time)
        } else {
            KeyboardTarget::key(&self.surface, seat, data, key, state, serial, time)
        }
    }
    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        mods: ModifiersState,
        serial: Serial,
    ) {
        KeyboardTarget::modifiers(&self.surface, seat, data, mods, serial)
    }
}

pub fn set_keyboard_focus(state: &mut State, window: Option<&Window>) {
    if state.desktop.lock.active || state.auth_surface().is_some() {
        return;
    }
    if let Some(w) = window {
        if w.x11_surface().is_some_and(|x| x.is_override_redirect()) {
            return;
        }
        state.space.raise_element(w, true);
        if let (Some(x), Some(wm)) = (w.x11_surface(), state.xwm.as_mut()) {
            let _ = wm.raise_window(x);
        }
    }
    for w in state.space.elements() {
        if let Some(t) = w.toplevel() {
            w.set_activated(Some(w) == window);
            if t.is_initial_configure_sent() {
                t.send_pending_configure();
            }
        }
    }
    if state.desktop.active_task.as_ref() != window {
        state.desktop.active_task = window.cloned();
        state.publish_tasks();
    }
    let focus = window.and_then(|w| {
        Some(KeyboardFocus {
            surface: w.wl_surface()?.into_owned(),
            x11: w.x11_surface().cloned(),
        })
    });
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(state, focus, SERIAL_COUNTER.next_serial());
    }
}
pub fn refocus(state: &mut State) {
    let next = state
        .space
        .elements()
        .rev()
        .find(|w| !w.x11_surface().is_some_and(|x| x.is_override_redirect()))
        .cloned();
    set_keyboard_focus(state, next.as_ref());
}
