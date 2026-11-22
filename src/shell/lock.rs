//! Mask scanout and isolate input independently of the lifetime of the PAM locker.
use crate::state::State;
use smithay::{
    backend::renderer::element::Id,
    input::pointer::MotionEvent,
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
};
use std::{collections::HashSet, process::Child, time::Instant};

pub struct Lock {
    pub(super) gpui: Option<super::gpui_lock::Attempt>,
    reported: bool,
    pub(crate) seat_active: bool,
    pub(crate) preparing_sleep: bool,
    pub active: bool,
    pub mask: Id,
    pub shade: Id,
    pub arrow: smithay::backend::renderer::element::memory::MemoryRenderBuffer,
    pub since: u64,
    pub covered: HashSet<Output>,
    pub child: Option<Child>,
    pub last_input: Instant,
}
impl Lock {
    pub fn gpui_active(&self) -> bool {
        self.gpui.is_some()
    }
    pub fn new() -> Self {
        Self {
            active: false,
            reported: false,
            seat_active: true,
            preparing_sleep: false,
            gpui: None,
            mask: Id::new(),
            shade: Id::new(),
            arrow: crate::cursor::build_cursor(),
            since: 0,
            covered: HashSet::new(),
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
        l.last_input = Instant::now();
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
        if self.desktop.lock.seat_active && !self.desktop.lock.preparing_sleep {
            match std::env::current_exe() {
                Ok(path) => {
                    self.supervise_gpui_locker(path.with_file_name("mundane-lock").into_os_string())
                }
                Err(error) => tracing::error!(%error,"cannot locate locker"),
            }
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
            if !self.desktop.lock.reported {
                self.report_lock(b"L");
                self.desktop.lock.reported = true;
            }
        }
    }
    pub(super) fn finish_unlock(&mut self) {
        self.report_lock(b"U");
        self.shell_message(&serde_json::json!({"locked":false}));
        self.desktop.lock.active = false;
        self.publish_tasks();
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
        self.desktop.lock.gpui.as_ref()?.ready(o)
    }
    pub fn lock_frame_recipient(&self, output: &Output) -> Option<WlSurface> {
        self.desktop.lock.gpui.as_ref()?.configured(output)
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
