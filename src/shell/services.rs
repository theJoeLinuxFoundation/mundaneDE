//! Only the session-launched helpers receive their privileged globals. Other
//! connections from the same uid, including a helper's children, are ordinary.
use crate::state::{ClientState, State};
use smithay::{
    desktop::{PopupKind, PopupManager},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Client, DisplayHandle},
    utils::{Logical, Rectangle},
    wayland::{
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        seat::WaylandFocus,
        selection::{
            ext_data_control::{DataControlHandler, DataControlState},
            primary_selection::PrimarySelectionState,
        },
        virtual_keyboard::VirtualKeyboardManagerState,
    },
};
use std::{
    os::unix::{io::OwnedFd, net::UnixStream},
    process::{Child, Command, Stdio},
    sync::Arc,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    #[default]
    Application,
    InputMethod,
    ClipboardManager,
}

fn role(client: &Client) -> Role {
    client
        .get_data::<ClientState>()
        .map(|data| data.role)
        .unwrap_or_default()
}
pub fn globals(dh: &DisplayHandle, primary: &PrimarySelectionState) -> DataControlState {
    InputMethodManagerState::new::<State, _>(dh, |c| role(c) == Role::InputMethod);
    VirtualKeyboardManagerState::new::<State, _>(dh, |c| role(c) == Role::InputMethod);
    DataControlState::new::<State, _>(dh, Some(primary), |c| role(c) == Role::ClipboardManager)
}
pub struct Helper(Child);
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn launch(dh: &DisplayHandle, on_exit: &calloop::ping::Ping) -> anyhow::Result<Vec<Helper>> {
    let mut children = Vec::new();
    for (key, role) in [
        ("MUNDANE_IBUS_PANEL", Role::InputMethod),
        ("MUNDANE_CLIPBOARD_MANAGER", Role::ClipboardManager),
    ] {
        let Some(program) = std::env::var_os(key) else {
            continue;
        };
        let (server, child) = UnixStream::pair()?;
        dh.clone().insert_client(
            server,
            Arc::new(ClientState {
                role,
                compositor_state: Default::default(),
                on_exit: on_exit.clone(),
            }),
        )?;
        let mut cmd = Command::new(program);
        if role == Role::InputMethod {
            cmd.arg("--enable-wayland-im");
        }
        cmd.stdin(Stdio::from(OwnedFd::from(child)))
            .env("WAYLAND_SOCKET", "0")
            .env("GDK_BACKEND", "wayland");
        children.push(Helper(cmd.spawn()?));
    }
    Ok(children)
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.clipboard_control
    }
}
impl InputMethodHandler for State {
    fn new_popup(&mut self, s: PopupSurface) {
        self.popup_repositioned(s.clone());
        if let Err(e) = self.popups.track_popup(PopupKind::InputMethod(s)) {
            tracing::warn!(%e,"cannot map input method popup");
        }
    }
    fn dismiss_popup(&mut self, s: PopupSurface) {
        if let Some(parent) = s.get_parent().map(|p| p.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::InputMethod(s));
        }
    }
    fn popup_repositioned(&mut self, s: PopupSurface) {
        let caret = s.text_input_rectangle();
        let mut pos = caret.loc;
        pos.y += caret.size.h;
        if let Some(parent) = s.get_parent() {
            if let Some(window) = self.wm.window_for_surface(&parent.surface) {
                if let Some(loc) = self.space.element_location(&window) {
                    let origin = loc - window.geometry().loc;
                    let size =
                        smithay::desktop::utils::bbox_from_surface_tree(s.wl_surface(), (0, 0))
                            .size;
                    let output = self.output_logical_size();
                    if origin.y + pos.y + size.h > output.h {
                        pos.y = caret.loc.y - size.h;
                    }
                    pos.x = (origin.x + pos.x).clamp(0, (output.w - size.w).max(0)) - origin.x;
                    pos.y = (origin.y + pos.y).clamp(0, (output.h - size.h).max(0)) - origin.y;
                }
            }
        }
        s.set_location(pos);
    }
    fn parent_geometry(&self, surface: &WlSurface) -> Rectangle<i32, Logical> {
        self.space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .map(|w| w.geometry())
            .unwrap_or_default()
    }
}
smithay::delegate_ext_data_control!(State);
smithay::delegate_input_method_manager!(State);
smithay::delegate_virtual_keyboard_manager!(State);
