//! Mask scanout and isolate input independently of the lifetime of the PAM locker.
use crate::{
    shell::services::{self, Role},
    state::State,
};
use smithay::{
    backend::renderer::element::Id,
    input::pointer::MotionEvent,
    output::Output,
    reexports::wayland_server::{
        protocol::{wl_output::WlOutput, wl_surface::WlSurface},
        DisplayHandle,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
};
use std::{
    collections::{HashMap, HashSet},
    process::Child,
    time::Instant,
};

pub struct Lock {
    manager: SessionLockManagerState,
    pub(super) gpui: Option<super::gpui_lock::Attempt>,
    reported: bool,
    pub(crate) seat_active: bool,
    pub(crate) preparing_sleep: bool,
    pub active: bool,
    pub mask: Id,
    pub since: u64,
    pub covered: HashSet<Output>,
    confirmation: Option<SessionLocker>,
    pub surfaces: HashMap<Output, LockSurface>,
    pub child: Option<Child>,
    pub last_input: Instant,
}
impl Lock {
    pub fn new(dh: &DisplayHandle) -> Self {
        Self {
            manager: SessionLockManagerState::new::<State, _>(dh, |c| {
                services::role(c) == Role::Locker
            }),
            active: false,
            reported: false,
            seat_active: true,
            preparing_sleep: false,
            gpui: None,
            mask: Id::new(),
            since: 0,
            covered: HashSet::new(),
            confirmation: None,
            surfaces: HashMap::new(),
            child: None,
            last_input: Instant::now(),
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
impl State {
    pub fn begin_lock(&mut self) {
        self.desktop.pending_keys.clear();
        if self.desktop.lock.active {
            return;
        }
        self.finish_switcher(false);
        self.revoke_capture(None);
        self.shell_message(&serde_json::json!({"locked":true}));
        self.generation += 1;
        let l = &mut self.desktop.lock;
        l.active = true;
        l.reported = false;
        l.since = self.generation;
        l.mask = Id::new();
        l.covered.clear();
        self.move_state = None;
        self.resize = None;
        self.window_menu = None;
        let serial = SERIAL_COUNTER.next_serial();
        let kb = self.seat.get_keyboard().unwrap();
        kb.unset_grab(self);
        kb.set_focus(self, None, serial);
        let p = self.seat.get_pointer().unwrap();
        p.unset_grab(self, serial, 0);
        p.motion(
            self,
            None,
            &MotionEvent {
                location: self.cursor_location,
                serial,
                time: 0,
            },
        );
        p.frame(self);
        self.ensure_locker();
    }
    pub fn ensure_locker(&mut self) {
        if !self.desktop.lock.active {
            return;
        }
        if let Some(path) = std::env::var_os("MUNDANE_GPUI_LOCKER") {
            if self.desktop.lock.seat_active && !self.desktop.lock.preparing_sleep {
                self.supervise_gpui_locker(path);
            }
            return;
        }
        if let Some(c) = &mut self.desktop.lock.child {
            if matches!(c.try_wait(), Ok(None)) {
                return;
            }
        }
        self.desktop.lock.child = None;
        self.desktop.lock.surfaces.clear();
        self.desktop.lock.confirmation = None;
        self.generation += 1;
        let mut cmd = std::process::Command::new(
            std::env::var_os("MUNDANE_LOCKER").unwrap_or_else(|| "swaylock".into()),
        );
        cmd.args(["--color", "182536", "--show-failed-attempts"]);
        match self.trusted_child(Role::Locker, cmd) {
            Ok(child) => self.desktop.lock.child = Some(child),
            Err(e) => tracing::error!(%e,"cannot start PAM locker; session stays covered"),
        }
    }
    pub fn lock_frame(&mut self, o: &Output, generation: u64) {
        if self.desktop.lock.active && generation >= self.desktop.lock.since {
            self.desktop.lock.covered.insert(o.clone());
            self.confirm_lock();
        }
    }
    fn confirm_lock(&mut self) {
        if self
            .space
            .outputs()
            .all(|o| self.desktop.lock.covered.contains(o))
        {
            if !self.desktop.lock.reported && std::env::var_os("MUNDANE_GPUI_LOCKER").is_some() {
                self.report_lock(b"L");
                self.desktop.lock.reported = true;
            }
            if let Some(c) = self.desktop.lock.confirmation.take() {
                c.lock();
                self.report_lock(b"L");
            }
        }
    }
    pub(super) fn finish_unlock(&mut self) {
        self.report_lock(b"U");
        self.shell_message(&serde_json::json!({"locked":false}));
        self.desktop.lock.active = false;
        self.publish_tasks();
        self.desktop.lock.surfaces.clear();
        self.desktop.lock.last_input = Instant::now();
        self.generation += 1;
        crate::focus::refocus(self);
    }
    fn report_lock(&self, msg: &[u8]) {
        use std::io::Write;
        if let Some(mut s) = self.session_channel.as_ref() {
            let _ = s.write_all(msg);
        }
    }
    pub fn lock_focus(&self) -> Option<WlSurface> {
        self.space
            .output_under(self.cursor_location)
            .next()
            .and_then(|o| self.lock_surface(o))
            .or_else(|| self.space.outputs().find_map(|o| self.lock_surface(o)))
    }
    pub fn lock_surface(&self, o: &Output) -> Option<WlSurface> {
        if let Some(attempt) = &self.desktop.lock.gpui {
            return attempt.ready(o);
        }
        self.desktop
            .lock
            .surfaces
            .get(o)
            .filter(|s| s.alive())
            .map(|s| s.wl_surface().clone())
    }
    pub fn lock_frame_recipient(&self, output: &Output) -> Option<WlSurface> {
        if let Some(a) = &self.desktop.lock.gpui {
            a.configured(output)
        } else {
            self.lock_surface(output)
        }
    }
    pub fn lock_pointer(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let o = self.space.output_under(pos).next()?;
        let s = self.lock_surface(o)?;
        Some((s, self.space.output_geometry(o)?.loc.to_f64()))
    }
}
impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.desktop.lock.manager
    }
    fn lock(&mut self, confirmation: SessionLocker) {
        if self.desktop.lock.active {
            self.desktop.lock.confirmation = Some(confirmation);
            self.confirm_lock();
        }
    }
    fn unlock(&mut self) {
        if self.desktop.lock.active && std::env::var_os("MUNDANE_GPUI_LOCKER").is_none() {
            self.finish_unlock();
        }
    }
    fn new_surface(&mut self, s: LockSurface, o: WlOutput) {
        if !self.desktop.lock.active {
            return;
        }
        if let Some(o) = Output::from_resource(&o) {
            if let Some(geo) = self.space.output_geometry(&o) {
                s.with_pending_state(|pending| {
                    pending.size = Some((geo.size.w as u32, geo.size.h as u32).into())
                });
                s.send_configure();
                self.desktop.lock.surfaces.insert(o, s);
                if let Some(f) = self.lock_focus() {
                    self.layer_focus(&f);
                }
                self.generation += 1;
            }
        }
    }
}
smithay::delegate_session_lock!(State);
