//! Keep one stacking-order snapshot for the duration of an Alt+Tab chord.
use crate::state::State;
use smithay::{desktop::Window, input::keyboard::ModifiersState, wayland::seat::WaylandFocus};
use xkbcommon::xkb::keysyms;

pub struct Switcher {
    windows: Vec<Window>,
    selected: usize,
}

impl State {
    pub fn finish_switcher(&mut self, activate: bool) {
        if let Some(switcher) = self.desktop.switcher.take() {
            self.shell_message(&serde_json::json!({"switcher":null}));
            let w = &switcher.windows[switcher.selected];
            if activate && self.desktop.tasks.contains_key(w) {
                if self.minimized.contains(w) {
                    self.restore_window(w);
                } else {
                    crate::focus::set_keyboard_focus(self, Some(w));
                }
            }
        }
    }

    pub fn switcher_key(&mut self, sym: u32, mods: &ModifiersState, pressed: bool) -> bool {
        if self.desktop.switcher.is_some() {
            if !mods.alt {
                self.finish_switcher(true);
                return false; // Deliver the Alt release to clear client modifier state.
            }
            if pressed && sym == keysyms::KEY_Escape {
                self.finish_switcher(false);
                return true;
            }
            if pressed
                && !xkbcommon::xkb::Keysym::new(sym).is_modifier_key()
                && !matches!(sym, keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab)
            {
                return true;
            }
        }
        if !pressed
            || !mods.alt
            || mods.logo
            || mods.ctrl
            || !matches!(sym, keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab)
        {
            return false;
        }
        if self.desktop.switcher.is_none() {
            let kb = self.seat.get_keyboard().unwrap();
            if kb.is_grabbed() || self.desktop.showing_desktop || self.window_menu.is_some() {
                return false;
            }
            let windows: Vec<_> = self
                .space
                .elements()
                .rev()
                .chain(self.minimized.iter().rev())
                .filter(|w| {
                    self.desktop
                        .tasks
                        .get(*w)
                        .is_some_and(|t| t.workspace == self.desktop.workspace)
                })
                .cloned()
                .collect();
            let focused = kb.current_focus();
            // Dialogs on exclusive shell layers (including authentication) retain input.
            let selected = if let Some(f) = focused {
                let Some(i) = windows
                    .iter()
                    .position(|w| w.wl_surface().as_deref() == Some(&f.surface))
                else {
                    return false;
                };
                i
            } else if windows.is_empty() {
                return false;
            } else if mods.shift {
                0
            } else {
                windows.len() - 1
            };
            self.desktop.switcher = Some(Switcher { windows, selected });
        }
        let switcher = self.desktop.switcher.as_mut().unwrap();
        let count = switcher.windows.len();
        let step = if mods.shift { count - 1 } else { 1 };
        switcher.selected = (switcher.selected + step) % count;
        let selected = switcher.selected;
        let start = selected.saturating_sub(2).min(count.saturating_sub(5));
        let titles: Vec<String> = switcher
            .windows
            .iter()
            .skip(start)
            .take(5)
            .map(|w| {
                let handle = &self.desktop.tasks[w].handle;
                let mut name = handle.title();
                if name.is_empty() {
                    name = handle.app_id();
                }
                name.chars().take(120).collect()
            })
            .collect();
        let output = self
            .space
            .output_under(self.cursor_location)
            .next()
            .map(|o| o.name());
        self.shell_message(&serde_json::json!({
            "switcher": titles, "selected": selected, "start": start, "count": count, "output": output
        }));
        true
    }

    pub fn switcher_lost_window(&mut self, w: &Window) {
        if self
            .desktop
            .switcher
            .as_ref()
            .is_some_and(|s| s.windows.contains(w))
        {
            self.finish_switcher(false);
        }
    }
}
