//! Publish only committed placement from the seat's active text-input to the shell.
use crate::state::State;
use smithay::{
    reexports::{
        wayland_protocols::wp::text_input::zv3::server::{
            zwp_text_input_manager_v3::ZwpTextInputManagerV3,
            zwp_text_input_v3::{Request, ZwpTextInputV3},
        },
        wayland_server::{backend::ClientId, Client, DataInit, Dispatch, DisplayHandle},
    },
    utils::{Logical, Rectangle},
    wayland::text_input::{TextInputManagerState, TextInputSeat, TextInputUserData},
};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Caret {
    pending: HashMap<ZwpTextInputV3, Rectangle<i32, Logical>>,
    last: Option<[i32; 4]>,
}
impl State {
    pub(crate) fn clear_caret(&mut self) {
        self.desktop.caret.pending.clear();
        if self.desktop.caret.last.take().is_some() {
            self.shell_message(&serde_json::json!({"hide_candidates":true}));
        }
    }
}
smithay::reexports::wayland_server::delegate_global_dispatch!(State:[ZwpTextInputManagerV3:()]=>TextInputManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State:[ZwpTextInputManagerV3:()]=>TextInputManagerState);
impl Dispatch<ZwpTextInputV3, TextInputUserData> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ZwpTextInputV3,
        request: Request,
        data: &TextInputUserData,
        dh: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let commit = matches!(request, Request::Commit);
        if let Request::SetCursorRectangle {
            x,
            y,
            width,
            height,
        } = request
        {
            state.desktop.caret.pending.insert(
                resource.clone(),
                Rectangle::new((x, y).into(), (width.max(0), height.max(0)).into()),
            );
        }
        <TextInputManagerState as Dispatch<_, _, Self>>::request(
            state, client, resource, request, data, dh, init,
        );
        if !commit || state.desktop.lock.active || state.auth_surface().is_some() {
            return;
        }
        let mut current = None;
        state
            .seat
            .text_input()
            .with_active_text_input(|input, surface| {
                current = Some((input.clone(), surface.clone()));
            });
        match current {
            Some((input, surface)) if input == *resource => {
                let Some(rect) = state.desktop.caret.pending.get(resource) else {
                    return;
                };
                let Some(window) = state.wm.window_for_surface(&surface) else {
                    return;
                };
                if let Some(loc) = state.space.element_location(&window) {
                    let pos = loc - window.geometry().loc + rect.loc;
                    let v = [pos.x, pos.y, rect.size.w, rect.size.h];
                    if state.desktop.caret.last != Some(v) {
                        state.desktop.caret.last = Some(v);
                        state.shell_message(&serde_json::json!({"caret":v}));
                    }
                }
            }
            None => state.clear_caret(),
            _ => {}
        }
    }
    fn destroyed(state: &mut Self, id: ClientId, r: &ZwpTextInputV3, d: &TextInputUserData) {
        state.desktop.caret.pending.remove(r);
        <TextInputManagerState as Dispatch<_, _, Self>>::destroyed(state, id, r, d);
    }
}
