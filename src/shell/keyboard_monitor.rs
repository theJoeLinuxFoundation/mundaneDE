//! The trusted shell supplies the subscription belonging to its Orca child.
use std::collections::{HashMap, HashSet};

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyboardMonitor {
    pub subscription: String,
    all: bool,
    modifiers: HashSet<u32>,
    keys: HashSet<(u32, u32)>,
    #[serde(skip)]
    held: HashMap<u32, u32>,
}

impl KeyboardMonitor {
    pub fn filter(&mut self, pressed: bool, code: u32, sym: u32, mask: u32) -> bool {
        if !pressed {
            return self.held.remove(&code).is_some();
        }
        let grab = self.all
            || self.modifiers.contains(&sym)
            || self.keys.contains(&(sym, mask))
            || self.held.values().any(|key| self.modifiers.contains(key));
        if grab {
            self.held.insert(code, sym);
        }
        grab
    }

    pub fn configure(&mut self, mut next: Self) {
        if self.subscription == next.subscription {
            next.held = std::mem::take(&mut self.held);
        }
        *self = next;
    }
}

use crate::{focus::KeyboardFocus, state::State};
use smithay::{
    backend::input::{KeyState, Keycode},
    utils::Serial,
};

pub struct PendingKey {
    code: Keycode,
    state: KeyState,
    serial: Serial,
    time: u32,
    modifiers_changed: bool,
    focus: Option<KeyboardFocus>,
}
impl State {
    pub fn defer_reader_key(
        &mut self,
        code: Keycode,
        state: KeyState,
        serial: Serial,
        time: u32,
        modifiers_changed: bool,
    ) {
        let focus = self.seat.get_keyboard().unwrap().current_focus();
        self.desktop.pending_keys.push_back(PendingKey {
            code,
            state,
            serial,
            time,
            modifiers_changed,
            focus,
        });
        // A stalled or disappearing accessibility service must not stop seat input.
        self.handle
            .insert_source(
                calloop::timer::Timer::from_duration(std::time::Duration::from_millis(50)),
                move |_, _, state| {
                    state.forward_reader_keys(serial);
                    calloop::timer::TimeoutAction::Drop
                },
            )
            .ok();
    }
    pub fn forward_reader_keys(&mut self, through: Serial) {
        let Some(index) = self
            .desktop
            .pending_keys
            .iter()
            .position(|k| k.serial == through)
        else {
            return;
        };
        let keyboard = self.seat.get_keyboard().unwrap();
        for _ in 0..=index {
            let k = self.desktop.pending_keys.pop_front().unwrap();
            if !self.desktop.lock.active && keyboard.current_focus() == k.focus {
                keyboard.input_forward(
                    self,
                    k.code,
                    k.state,
                    k.serial,
                    k.time,
                    k.modifiers_changed,
                );
            }
        }
    }
}
