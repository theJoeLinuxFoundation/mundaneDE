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
    Shell,
    Locker,
    Capture,
    Screenshot,
    InputMethod,
    ClipboardManager,
}

pub(crate) fn role(client: &Client) -> Role {
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
                context: None,
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
                    let output = self.window_bounds(&window);
                    if origin.y + pos.y + size.h > output.loc.y + output.size.h {
                        pos.y = caret.loc.y - size.h;
                    }
                    pos.x = (origin.x + pos.x)
                        .clamp(output.loc.x, output.loc.x + (output.size.w - size.w).max(0))
                        - origin.x;
                    pos.y = (origin.y + pos.y)
                        .clamp(output.loc.y, output.loc.y + (output.size.h - size.h).max(0))
                        - origin.y;
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

impl State {
    pub fn trusted_child(&self, role: Role, mut cmd: Command) -> anyhow::Result<Child> {
        let (server, child) = UnixStream::pair()?;
        self.display_handle.clone().insert_client(
            server,
            Arc::new(ClientState {
                role,
                context: None,
                compositor_state: Default::default(),
                on_exit: self.client_exit.clone(),
            }),
        )?;
        Ok(cmd
            .stdin(Stdio::from(OwnedFd::from(child)))
            .env("WAYLAND_SOCKET", "0")
            .spawn()?)
    }
    fn start_panel(&mut self) -> anyhow::Result<()> {
        use std::os::unix::net::UnixDatagram;
        let Some(path) = std::env::var_os("MUNDANE_PANEL") else {
            return Ok(());
        };
        let (server, client) = UnixDatagram::pair()?;
        server.set_nonblocking(true)?;
        let mut cmd = Command::new(path);
        cmd.stdout(Stdio::from(OwnedFd::from(client)))
            .env("GDK_BACKEND", "wayland");
        tracing::debug!(target: "mundane_shell_frames", "launch shell");
        let child = self.trusted_child(Role::Shell, cmd)?;
        let source = server.try_clone()?;
        let pid = child.id();
        if let Some(old) = self.desktop.panel_source.take() {
            self.handle.remove(old);
        }
        let token = self
            .handle
            .insert_source(
                calloop::generic::Generic::new(
                    source,
                    calloop::Interest::READ,
                    calloop::Mode::Level,
                ),
                move |_, sock, state| {
                    if state.desktop.panel.as_ref().map(|c| c.id()) != Some(pid) {
                        return Ok(calloop::PostAction::Remove);
                    }
                    if let Err(error) = state.panel_datagram(sock) {
                        tracing::warn!(%error, "shell control message");
                    }
                    Ok(calloop::PostAction::Continue)
                },
            )
            .map_err(|e| anyhow::anyhow!("shell control: {e}"))?;
        self.desktop.panel_source = Some(token);
        self.desktop.channel = Some(server);
        self.desktop.watch_tasks = false;
        self.desktop.panel = Some(child);
        self.desktop.monitor=Default::default();
        if self.desktop.lock.active {
            self.shell_message(&serde_json::json!({"locked": true}));
        }
        Ok(())
    }
    pub fn supervise_shell(&mut self) -> anyhow::Result<()> {
        if std::env::var_os("MUNDANE_PANEL").is_none() {
            return Ok(());
        }
        self.start_panel()?;
        self.handle
            .insert_source(
                calloop::timer::Timer::from_duration(std::time::Duration::from_secs(1)),
                |_, _, state| {
                    let dead = state
                        .desktop
                        .panel
                        .as_mut()
                        .is_none_or(|c| !matches!(c.try_wait(), Ok(None)));
                    if dead {
                        state.revoke_capture(None);
                        if let Err(e) = state.start_panel() {
                            tracing::error!(%e,"restart panel");
                        }
                    }
                    if let Some(c) = &mut state.desktop.lock.child {
                        let _ = c.try_wait();
                    }
                    state.update_inhibitors();
                    if state.desktop.lock.active {
                        state.ensure_locker();
                    } else if state.desktop.lock.last_input.elapsed().as_secs() >= 600
                        && !state.idle_inhibited()
                    {
                        state.begin_lock();
                    }
                    calloop::timer::TimeoutAction::ToDuration(std::time::Duration::from_secs(1))
                },
            )
            .map_err(|e| anyhow::anyhow!("shell supervision: {e}"))?;
        Ok(())
    }
}
