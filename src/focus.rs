//! Keyboard focus helpers.
//!
//! The MVP uses click-to-focus plus focus-the-newly-mapped-window. With
//! `SeatHandler::KeyboardFocus = WlSurface`, focus state lives in the seat's keyboard;
//! these helpers resolve a [`Window`] to its toplevel surface and update that focus.

use smithay::desktop::Window;
use smithay::utils::SERIAL_COUNTER;

use crate::state::State;

/// Focus `window` (raising it) or clear focus when `None`.
pub fn set_keyboard_focus(state: &mut State, window: Option<&Window>) {
    if let Some(w) = window {
        state.space.raise_element(w, true);
    }
    let surface = window
        .and_then(|w| w.toplevel())
        .map(|t| t.wl_surface().clone());
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(state, surface, SERIAL_COUNTER.next_serial());
    }
}

/// Move focus to the top-most remaining window (used after a window closes).
pub fn refocus(state: &mut State) {
    let next = state.space.elements().last().cloned();
    set_keyboard_focus(state, next.as_ref());
}
