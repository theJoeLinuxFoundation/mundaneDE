//! One private connection and a new set of output tickets for each locker process.
use super::services::{role, Role};
use crate::state::State;
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    output::Output,
    reexports::{
        rustix::event::{poll, PollFd, PollFlags, Timespec},
        rustix::net::{socketpair, AddressFamily, SocketFlags, SocketType},
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{backend::ClientId, protocol::wl_surface::WlSurface, Client, Resource},
    },
    utils::{Logical, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData},
    },
};
use std::{
    collections::HashMap,
    ffi::OsString,
    io::Read,
    os::unix::net::UnixDatagram,
    process::{Command, Stdio},
};

pub(super) fn locker_epoch(s: &WlSurface) -> Option<u64> {
    match role(&s.client()?) {
        Role::GpuiLocker(e) => Some(e),
        _ => None,
    }
}
struct Slot {
    ticket: String,
    size: Size<i32, Logical>,
    scale: f64,
    window: Option<ToplevelSurface>,
    used: bool,
}
impl Slot {
    fn claim(&mut self, ticket: &str) -> bool {
        if self.used || self.ticket != ticket {
            return false;
        }
        self.used = true;
        true
    }
}
pub(super) struct Attempt {
    epoch: u64,
    client: ClientId,
    channel: UnixDatagram,
    source: calloop::RegistrationToken,
    slots: HashMap<Output, Slot>,
    presentation: Option<(Option<Output>, bool)>,
}
impl Attempt {
    fn owns(&self, active: bool, client: Option<&Client>) -> bool {
        active
            && client
                .is_some_and(|c| role(c) == Role::GpuiLocker(self.epoch) && c.id() == self.client)
    }
    fn completion(&self, epoch: u64, packet: &[u8]) -> bool {
        if epoch != self.epoch || packet != format!("authenticated/1 {epoch}").as_bytes() {
            return false;
        }
        let mut fds = [PollFd::new(&self.channel, PollFlags::RDHUP)];
        poll(&mut fds, Some(&Timespec::default())).is_ok()
            && !fds[0]
                .revents()
                .intersects(PollFlags::RDHUP | PollFlags::HUP | PollFlags::ERR)
    }
    pub(super) fn configured(&self, o: &Output) -> Option<WlSurface> {
        Some(self.slots.get(o)?.window.as_ref()?.wl_surface().clone())
    }
    pub(super) fn ready(&self, o: &Output) -> Option<WlSurface> {
        let slot = self.slots.get(o)?;
        let s = slot.window.as_ref()?.wl_surface();
        (s.is_alive()
            && with_renderer_surface_state(s, |data| data.surface_size()) == Some(Some(slot.size)))
        .then(|| s.clone())
    }
}
impl State {
    pub(crate) fn invalidate_lock_attempt(&mut self) {
        if self.desktop.lock.gpui.is_some() {
            self.retire_locker();
            self.desktop.lock.since = self.generation;
            self.desktop.lock.covered.clear();
        }
    }
    fn retire_locker(&mut self) {
        if let Some(a) = self.desktop.lock.gpui.take() {
            self.handle.remove(a.source);
        }
        if let Some(mut child) = self.desktop.lock.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.generation += 1;
    }
    pub(super) fn supervise_gpui_locker(&mut self, path: OsString) {
        if let Some(a) = &self.desktop.lock.gpui {
            if self
                .display_handle
                .backend_handle()
                .get_client_data(a.client.clone())
                .is_ok()
                && self
                    .desktop
                    .lock
                    .child
                    .as_mut()
                    .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
            {
                self.sync_lock_presentation(false);
                return;
            }
        }
        self.retire_locker();
        if let Err(e) = self.spawn_gpui_locker(path) {
            tracing::error!(%e,"locker unavailable; keeping mask");
        }
    }
    fn spawn_gpui_locker(&mut self, path: OsString) -> anyhow::Result<()> {
        let epoch = self.generation;
        let (server, peer) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        let server = UnixDatagram::from(server);
        server.set_nonblocking(true)?;
        let mut cmd = Command::new(path);
        cmd.stdout(Stdio::from(peer));
        let (child, client) = self.spawn_connected(Role::GpuiLocker(epoch), cmd)?;
        self.desktop.lock.child = Some(child);
        let source = self
            .handle
            .insert_source(
                calloop::generic::Generic::new(
                    server.try_clone()?,
                    calloop::Interest::READ,
                    calloop::Mode::Level,
                ),
                move |_, s, state| {
                    // An old calloop dispatch cannot operate on a newer attempt.
                    if state
                        .desktop
                        .lock
                        .gpui
                        .as_ref()
                        .is_none_or(|a| a.epoch != epoch)
                    {
                        return Ok(calloop::PostAction::Remove);
                    }
                    let mut buf = [0; 256];
                    match s.recv(&mut buf) {
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Ok(0) | Err(_) => state.retire_locker(),
                        Ok(n) => {
                            if n < buf.len()
                                && state
                                    .desktop
                                    .lock
                                    .gpui
                                    .as_ref()
                                    .is_some_and(|a| a.completion(epoch, &buf[..n]))
                            {
                                let unlock = state.live_prompt(epoch);
                                state.retire_locker();
                                if unlock {
                                    state.finish_unlock();
                                }
                            }
                        }
                    }
                    Ok(calloop::PostAction::Continue)
                },
            )
            .map_err(|e| anyhow::anyhow!("locker control: {e}"))?;
        self.desktop.lock.gpui = Some(Attempt {
            epoch,
            client: client.id(),
            channel: server,
            source,
            slots: HashMap::new(),
            presentation: None,
        });
        self.sync_lock_outputs();
        Ok(())
    }
    fn live_prompt(&mut self, epoch: u64) -> bool {
        self.desktop.lock.active
            && self.desktop.lock.gpui.as_ref().is_some_and(|a| {
                a.epoch == epoch
                    && self
                        .display_handle
                        .backend_handle()
                        .get_client_data(a.client.clone())
                        .is_ok()
                    && self.space.outputs().any(|o| a.ready(o).is_some())
            })
            && self
                .desktop
                .lock
                .child
                .as_mut()
                .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }
    pub(crate) fn sync_lock_outputs(&mut self) {
        let Some(a) = self.desktop.lock.gpui.as_mut() else {
            return;
        };
        let before = a.slots.len();
        a.slots
            .retain(|o, _| self.space.output_geometry(o).is_some());
        let mut changed = before != a.slots.len();
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let size = self.space.output_geometry(&output).unwrap().size;
            let scale = output.current_scale().fractional_scale();
            if a.slots
                .get(&output)
                .is_some_and(|slot| slot.size == size && slot.scale == scale)
            {
                continue;
            }
            let mut random = [0; 16];
            if std::fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut random))
                .is_err()
            {
                self.retire_locker();
                return;
            }
            let ticket = format!("mundane-lock-{:032x}", u128::from_be_bytes(random));
            a.slots.insert(
                output.clone(),
                Slot {
                    ticket,
                    size,
                    scale,
                    window: None,
                    used: false,
                },
            );
            changed = true;
        }
        self.sync_lock_presentation(changed);
    }
    pub(crate) fn sync_lock_presentation(&mut self, force: bool) {
        let Some(a) = self.desktop.lock.gpui.as_mut() else {
            return;
        };
        let active = self
            .space
            .output_under(self.cursor_location)
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned();
        let awake = self.desktop.lock.last_input.elapsed().as_secs() < 30;
        if force || a.presentation.as_ref() != Some(&(active.clone(), awake)) {
            let outputs: Vec<_> = a
                .slots
                .iter()
                .map(|(o, s)| {
                    serde_json::json!({
                        "ticket":s.ticket,"width":s.size.w,"height":s.size.h,"name":o.name(),
                        "prompt":awake && Some(o)==active.as_ref()
                    })
                })
                .collect();
            a.presentation = Some((active, awake));
            let packet =
                serde_json::json!({"version":1,"epoch":a.epoch,"outputs":outputs,"awake":awake})
                    .to_string();
            if a.channel.send(packet.as_bytes()).is_err() {
                self.retire_locker();
            }
        }
    }
    pub(super) fn lock_window_destroyed(&mut self, window: &ToplevelSurface) {
        if let Some(a) = self.desktop.lock.gpui.as_mut() {
            let before = a.slots.len();
            a.slots
                .retain(|_, slot| slot.window.as_ref() != Some(window));
            if before != a.slots.len() {
                self.sync_lock_outputs();
            }
        }
    }
    pub(super) fn commit_lock_window(&mut self, s: &WlSurface) {
        let Some(a) = self.desktop.lock.gpui.as_mut() else {
            return;
        };
        if !a.owns(self.desktop.lock.active, s.client().as_ref()) {
            return;
        }
        if a.slots
            .values()
            .any(|slot| slot.window.as_ref().is_some_and(|w| w.wl_surface() == s))
        {
            if let Some(f) = self.lock_focus() {
                if self
                    .seat
                    .get_keyboard()
                    .unwrap()
                    .current_focus()
                    .is_none_or(|focus| focus.surface != f)
                {
                    self.layer_focus(&f);
                }
            }
            return;
        }
        let Some(window) = self
            .xdg_shell_state
            .toplevel_surfaces()
            .iter()
            .find(|w| w.wl_surface() == s)
            .cloned()
        else {
            return;
        };
        let ticket = with_states(s, |d| {
            d.data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().ok()?.app_id.clone())
        });
        let Some(ticket) = ticket else {
            window.send_close();
            return;
        };
        let Some((o, slot)) = a.slots.iter_mut().find(|(_, slot)| ticket == slot.ticket) else {
            window.send_close();
            return;
        };
        if !slot.claim(&ticket) {
            window.send_close();
            return;
        }
        window.with_pending_state(|p| {
            p.size = Some(slot.size);
            p.states.set(xdg_toplevel::State::Activated);
            p.states.set(xdg_toplevel::State::Fullscreen);
        });
        with_states(s, |data| {
            smithay::wayland::fractional_scale::with_fractional_scale(data, |scale| {
                scale.set_preferred_scale(slot.scale)
            });
            smithay::wayland::compositor::send_surface_state(
                s,
                data,
                o.current_scale().integer_scale(),
                smithay::utils::Transform::Normal,
            );
        });
        window.send_configure();
        o.enter(s);
        slot.window = Some(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::wayland_server::Display;
    use std::{os::unix::net::UnixStream, sync::Arc};
    #[test]
    fn tickets_are_consumed_only_by_their_own_output() {
        let mut slot = Slot {
            ticket: "left".into(),
            size: (100, 100).into(),
            scale: 1.,
            window: None,
            used: false,
        };
        assert!(!slot.claim("removed"));
        assert!(!slot.claim("right"));
        assert!(slot.claim("left"));
        assert!(!slot.claim("left"));
    }
    #[test]
    fn lock_authority_requires_active_generation_live_peer_and_exact_connection() {
        let display = Display::<()>::new().unwrap();
        let mut handle = display.handle();
        let event_loop = calloop::EventLoop::<()>::try_new().unwrap();
        let (ping, receiver) = calloop::ping::make_ping().unwrap();
        let mut connect = |role| {
            let (server, peer) = UnixStream::pair().unwrap();
            let client = handle
                .insert_client(
                    server,
                    Arc::new(crate::state::ClientState {
                        context: None,
                        on_exit: ping.clone(),
                        compositor_state: Default::default(),
                        role,
                    }),
                )
                .unwrap();
            (client, peer)
        };
        let (locker, _live) = connect(Role::GpuiLocker(3));
        let source = event_loop
            .handle()
            .insert_source(receiver, |_, _, _| {})
            .unwrap();
        let (server, peer) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let mut attempt = Attempt {
            epoch: 3,
            client: locker.id(),
            channel: server.into(),
            source,
            slots: HashMap::new(),
            presentation: None,
        };
        assert!(attempt.owns(true, Some(&locker)));
        assert!(!attempt.owns(false, Some(&locker)));
        assert!(!attempt.owns(true, None));
        attempt.epoch += 1;
        assert!(!attempt.owns(true, Some(&locker)));
        attempt.epoch -= 1;
        for role in [
            Role::Application,
            Role::Shell,
            Role::InputMethod,
            Role::GpuiLocker(2),
            Role::GpuiLocker(3),
        ] {
            let (other, _peer) = connect(role);
            assert!(!attempt.owns(true, Some(&other)), "{role:?}");
        }
        assert!(!attempt.completion(2, b"authenticated/1 3"));
        assert!(!attempt.completion(3, b"authenticated/1 2"));
        for invalid in [
            b"unlock".as_slice(),
            b"authenticated/2 3",
            b"authenticated/1 3\0",
        ] {
            assert!(!attempt.completion(3, invalid));
        }
        assert!(attempt.completion(3, b"authenticated/1 3"));
        let peer = UnixDatagram::from(peer);
        peer.send(b"authenticated/1 3").unwrap();
        drop(peer);
        let mut pending = [0; 32];
        let n = attempt.channel.recv(&mut pending).unwrap();
        assert_eq!(&pending[..n], b"authenticated/1 3");
        assert!(!attempt.completion(3, &pending[..n]));
    }
}
