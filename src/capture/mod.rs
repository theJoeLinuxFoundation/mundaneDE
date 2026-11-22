//! Each capture connection belongs to one source picked in the trusted shell.
mod protocol;
mod render;
use crate::{
    shell::services::Role,
    state::{ClientState, State},
};
use smithay::{
    desktop::Window,
    output::Output,
    reexports::{
        rustix,
        wayland_server::{
            backend::{ClientId, DisconnectReason},
            protocol::wl_buffer::WlBuffer,
            DisplayHandle,
        },
    },
};
use std::{
    collections::HashMap,
    io::IoSliceMut,
    mem::MaybeUninit,
    os::unix::net::{UnixDatagram, UnixStream},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Output(String),
    Window(String),
}

#[derive(Default)]
pub struct Capture {
    grants: HashMap<ClientId, Source>,
    pending: Vec<Pending>,
    wake: crate::outputs::FrameWake,
}
struct Pending {
    source: Source,
    cursor: bool,
    buffer: WlBuffer,
    frame: protocol::Frame,
    region: smithay::utils::Rectangle<i32, smithay::utils::Physical>,
    after: Option<(u64, std::time::Instant)>,
}
impl Capture {
    pub fn new(dh: &DisplayHandle) -> Self {
        protocol::register(dh);
        Self::default()
    }
}
impl State {
    fn capture_output(&self, name: &str) -> Option<Output> {
        self.space.outputs().find(|o| o.name() == name).cloned()
    }
    fn capture_window(&self, id: &str) -> Option<Window> {
        self.desktop
            .tasks
            .iter()
            .find(|(_, t)| t.handle.identifier() == id)
            .map(|(w, _)| w.clone())
    }
    fn capture_size(
        &self,
        source: &Source,
    ) -> Option<smithay::utils::Size<i32, smithay::utils::Physical>> {
        match source {
            Source::Output(n) => {
                let o = self.capture_output(n)?;
                Some(o.current_transform().transform_size(o.current_mode()?.size))
            }
            Source::Window(id) => Some(self.capture_window(id)?.geometry().size.to_physical(1)),
        }
        .filter(|size| size.w > 0 && size.h > 0)
    }
    pub fn revoke_capture(&mut self, source: Option<&Source>) {
        let handle = self.display_handle.backend_handle();
        self.capture.grants.retain(|id, s| {
            if source.is_none_or(|requested| s == requested) {
                if handle.get_client_data(id.clone()).is_ok() {
                    handle.kill_client(id.clone(), DisconnectReason::ConnectionClosed);
                }
                false
            } else {
                true
            }
        });
        self.capture
            .pending
            .retain(|p| source.is_some_and(|s| s != &p.source));
    }
    pub fn panel_datagram(&mut self, socket: &UnixDatagram) -> anyhow::Result<()> {
        use rustix::net::{recvmsg, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags};
        let mut bytes = [0; 1024];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let msg = recvmsg(
            socket,
            &mut [IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC,
        )?;
        let command = std::str::from_utf8(&bytes[..msg.bytes])?;
        let Some(RecvAncillaryMessage::ScmRights(mut rights)) = ancillary.drain().next() else {
            self.shell_command(command);
            return Ok(());
        };
        let Some(fd) = rights.next() else {
            return Ok(());
        };
        let (role, source) = if let Some(n) = command.strip_prefix("capture-output ") {
            (Role::Capture, Source::Output(n.into()))
        } else if let Some(n) = command.strip_prefix("screenshot-output ") {
            (Role::Screenshot, Source::Output(n.into()))
        } else if let Some(id) = command.strip_prefix("capture-window ") {
            (Role::Capture, Source::Window(id.into()))
        } else {
            return Ok(());
        };
        if self.desktop.lock.active || self.capture_size(&source).is_none() {
            return Ok(());
        }
        let handle = self.display_handle.backend_handle();
        self.capture
            .grants
            .retain(|id, _| handle.get_client_data(id.clone()).is_ok());
        let client = self.display_handle.insert_client(
            UnixStream::from(fd),
            Arc::new(ClientState {
                role,
                context: None,
                on_exit: self.client_exit.clone(),
                compositor_state: Default::default(),
            }),
        )?;
        self.capture.grants.insert(client.id(), source);
        Ok(())
    }
}
