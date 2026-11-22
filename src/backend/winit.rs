//! The winit development backend.
//!
//! Runs the compositor inside a host window. A calloop event loop drives three sources —
//! the Wayland listening socket, the display fd (client request dispatch) and the winit
//! event source — and the per-iteration callback ticks the animation engine, syncs spring
//! positions into the [`Space`], renders, and sends frame callbacks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, InputBackend, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::input::keyboard::{keysyms, FilterResult};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::output::{Mode, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::presentation::Refresh;
use smithay::utils::{Logical, Physical, Point, Serial, Size, Transform, SERIAL_COUNTER};
use smithay::wayland::socket::ListeningSocketSource;

use crate::config::MundaneConfig;
use crate::state::{ClientState, MoveState, State};

/// Left mouse button (from `linux/input-event-codes.h`).
const BTN_LEFT: u32 = 0x110;

fn thread_cpu_time() -> Duration {
    use smithay::reexports::rustix::time::{clock_gettime, ClockId};
    let time = clock_gettime(ClockId::ThreadCPUTime);
    Duration::new(time.tv_sec as u64, time.tv_nsec as u32)
}

pub fn run_winit(config: MundaneConfig) -> anyhow::Result<()> {
    // NOTE: calloop's `InsertError<Source>` and winit/xkb error types embed non-Send/Sync
    // internals (Rc, NonNull<xkb_*>, channel Receivers), so they can't convert to
    // `anyhow::Error` via `?`. Render each to a string-based error with `map_err` instead.
    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().map_err(|e| anyhow::anyhow!("event loop init failed: {e}"))?;
    let display: Display<State> = Display::new()?;
    let dh = display.handle();

    use smithay::reexports::winit::{dpi::LogicalSize, window::Window};
    let size = LogicalSize::new(config.output.width.max(1), config.output.height.max(1));
    let attributes = Window::default_attributes()
        .with_title("mundaneDE")
        .with_inner_size(size);
    let (mut backend, winit) = winit::init_from_attributes::<GlesRenderer>(attributes)
        .map_err(|e| anyhow::anyhow!("winit init failed: {e}"))?;

    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let loop_signal = event_loop.get_signal();
    let mode = Mode {
        size: backend.window_size(),
        refresh: config.output.refresh.max(1000),
    };
    let mut state = State::new(
        &dh,
        config,
        "winit".into(),
        loop_signal,
        event_loop.handle(),
        socket_name.clone(),
    )?;
    let head = state.add_head(
        "winit".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "mundaneDE".into(),
            model: "Winit".into(),
        },
        mode,
        Transform::Flipped180,
    );
    state.advertise_dmabuf(backend.renderer(), None);
    let output = head.output;
    let mut damage_tracker =
        smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);
    let nested_output = output.clone();
    state.launch_helpers()?;
    let _x11_listener = crate::x11::listen(&state)?;

    let handle = event_loop.handle();

    handle
        .insert_source(socket, move |stream, _, state: &mut State| {
            if let Err(e) = state.display_handle.insert_client(
                stream,
                Arc::new(ClientState {
                    context: None,
                    on_exit: state.client_exit.clone(),
                    role: Default::default(),
                    compositor_state: Default::default(),
                }),
            ) {
                tracing::warn!("failed to add wayland client: {e}");
            }
        })
        .map_err(|e| anyhow::anyhow!("failed to insert socket source: {e}"))?;

    handle
        .insert_source(
            Generic::new(display, Interest::READ, CalloopMode::Level),
            |_, display, state: &mut State| {
                // SAFETY: we only dispatch clients; we never drop/close the display fd.
                unsafe {
                    display.get_mut().dispatch_clients(state)?;
                    state.generation += 1;
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("failed to insert display source: {e}"))?;

    handle
        .insert_source(winit, move |event, _, state: &mut State| match event {
            WinitEvent::Resized { size, .. } => {
                let refresh = state.config.output.refresh;
                nested_output.change_current_state(Some(Mode { size, refresh }), None, None, None);
                state.arrange_heads();
            }
            WinitEvent::Input(ev) => state.handle_input(ev),
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            WinitEvent::Redraw => state.generation += 1,
            WinitEvent::Focus(_) => {}
        })
        .map_err(|e| anyhow::anyhow!("failed to insert winit source: {e}"))?;

    // Hot-reload watcher: kept alive in this function's scope so the watcher and the
    // calloop source live for the event loop's lifetime.
    let _hot_reload = match crate::hot_reload::install(&handle) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("hot-reload disabled: {e}");
            None
        }
    };

    tracing::info!("mundaneDE winit backend running on WAYLAND_DISPLAY={socket_name}");

    let mut last_frame = Instant::now();
    let mut presented_size = None;
    let period = Duration::from_secs_f64(1000. / mode.refresh as f64);
    let mut deadline = Instant::now();
    let mut generation = 0;
    let mut wake = crate::outputs::FrameWake::default();
    let mut sequence = 0;
    event_loop.run(None, &mut state, |state| {
        state.import_buffers(backend.renderer());
        state.copy_capture_frames(backend.renderer());
        let now = Instant::now();
        let active = state.animation.has_active_animations();
        if (active || generation != state.generation) && now >= deadline {
            tracing::trace!(target:"mundane_frame","repaint");
            let dt = (now - last_frame).min(period * 3);
            last_frame = now;
            let start_cpu = tracing::enabled!(target: "mundane_frame", tracing::Level::TRACE).then(thread_cpu_time);
            state.animate(dt);
            let spring_cpu = start_cpu.map(|start| thread_cpu_time() - start);
            state.space.refresh();
            state.update_scales();
            match render(&mut backend,state,&output,&mut damage_tracker,&mut presented_size) {
                Ok(Some(mut feedback)) => {
                    sequence += 1;
                    state.lock_frame(&output,state.generation);
                    feedback.presented(state.clock.now(), Refresh::fixed(period), sequence, Kind::empty());
                    tracing::trace!(target: "mundane_frame",
                        frame = sequence,
                        spring_us = spring_cpu.unwrap_or_default().as_micros() as u64,
                        cpu_us = start_cpu.map(|start| thread_cpu_time() - start).unwrap_or_default().as_micros() as u64,
                        submit_us = now.elapsed().as_micros() as u64,
                        time_us = state.start_time.elapsed().as_micros() as u64,
                        "submitted"
                    );
                }
                Ok(None)=>{}
                Err(e)=>{presented_size=None;tracing::error!("render error: {e}");}
            }
            generation = state.generation;
            state.send_frames(&output);
            deadline = if now.duration_since(deadline) >= period { now + period } else { deadline + period };
        }
        if state.animation.has_active_animations() || generation != state.generation {
            wake.schedule(&state.handle,deadline);
        }
        state.popups.cleanup();
        state.space.refresh();
        let _ = state.display_handle.flush_clients();
    })
    .map_err(|e| anyhow::anyhow!("event loop error: {e}"))?;

    Ok(())
}

fn render(
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    state: &mut State,
    output: &smithay::output::Output,
    damage_tracker: &mut smithay::backend::renderer::damage::OutputDamageTracker,
    presented_size: &mut Option<Size<i32, Physical>>,
) -> anyhow::Result<Option<smithay::desktop::utils::OutputPresentationFeedback>> {
    use smithay::input::pointer::CursorImageStatus;
    let draw_cursor = matches!(state.cursor_status, CursorImageStatus::Surface(_));
    backend
        .window()
        .set_cursor_visible(matches!(state.cursor_status, CursorImageStatus::Named(_)));
    if let CursorImageStatus::Named(icon) = state.cursor_status {
        backend.window().set_cursor(icon);
    }
    let size = backend.window_size();
    // A new, resized or recreated surface has no usable damage history. Querying
    // BUFFER_AGE_EXT before the first bind can itself raise EGL_BAD_SURFACE.
    let age = if *presented_size == Some(size) && backend.egl_surface().is_current() {
        let age = backend.buffer_age().unwrap_or(0);
        tracing::trace!(age, "queried nested EGL buffer age");
        age
    } else {
        0
    };
    let (damage, states) = {
        let (renderer, mut framebuffer) =
            backend.bind().map_err(|e| anyhow::anyhow!("bind: {e}"))?;
        // Named cursors belong to the host. Render application cursor surfaces ourselves.
        let (elements, clear_color) =
            crate::render::collect_elements(renderer, state, output, draw_cursor);
        let res = damage_tracker
            .render_output(renderer, &mut framebuffer, age, &elements, clear_color)
            .map_err(|e| anyhow::anyhow!("render_output: {e:?}"))?;
        (res.damage.cloned(), res.states)
    };

    if let Some(damage) = damage {
        backend
            .submit(Some(&damage))
            .map_err(|e| anyhow::anyhow!("submit: {e}"))?;
        *presented_size = Some(size);
        return Ok(Some(state.feedback(output, &states)));
    }
    Ok(None)
}

impl State {
    /// Dispatch an input event from any backend: keybindings, click-to-focus, and interactive
    /// move. Generic over [`InputBackend`] so the winit (absolute pointer) and DRM (libinput,
    /// relative pointer) backends share one handler.
    pub fn handle_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        self.generation += 1;
        self.input_activity();
        match event {
            InputEvent::Keyboard { event } => {
                let keyboard = self.seat.get_keyboard().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                let press = event.state() == KeyState::Pressed;
                if self.desktop.lock.active {
                    let target = self.lock_focus().map(crate::focus::KeyboardFocus::native);
                    if keyboard.current_focus() != target {
                        keyboard.set_focus(self, target, serial);
                    }
                }
                if press && !self.desktop.lock.active {
                    self.record_input(serial, keyboard.current_focus().map(|f| f.surface));
                }
                let mut announced=false;
                let (filtered,mods_changed)=keyboard.input_intercept(
                    self,
                    event.key_code(),
                    event.state(),
                    |state, mods, handle| {
                        let sym = handle.modified_sym().raw();
                        if let Some(switch_vt) = &state.switch_vt {
                            if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12)
                                .contains(&sym)
                            {
                                if press {
                                    switch_vt((sym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32);
                                }
                                return FilterResult::Intercept(());
                            }
                        }
                        if state.desktop.lock.active {
                            return FilterResult::Forward;
                        }
                        if press && mods.logo && mods.alt && sym==keysyms::KEY_s {
                            state.shell_message(&serde_json::json!({"screen_reader":"toggle"}));
                            return FilterResult::Intercept(());
                        }
                        if !state.desktop.monitor.subscription.is_empty() {
                            let mask=mods.serialized.depressed|mods.serialized.latched|mods.serialized.locked;
                            let code=event.key_code().raw();
                            let grab=state.desktop.monitor.filter(press,code,sym,mask);
                            announced=true;
                            state.shell_message(&serde_json::json!({"serial":u32::from(serial),"subscription":state.desktop.monitor.subscription,
                                "key":[!press,mask,sym,xkbcommon::xkb::keysym_to_utf32(handle.modified_sym()),code]}));
                            if grab {return FilterResult::Intercept(());}
                        }
                        if state.window_menu.is_some() {
                            if press {
                                state.menu_key(sym);
                            }
                            return FilterResult::Intercept(());
                        }
                        if press && state.desktop.panel.is_some() {
                            let delta = match sym {
                                keysyms::KEY_XF86AudioRaiseVolume => Some("5%+"),
                                keysyms::KEY_XF86AudioLowerVolume => Some("5%-"),
                                _ => None,
                            };
                            if let Some(d) = delta {
                                state.shell_message(&serde_json::json!({"osd":d}));
                                return FilterResult::Intercept(());
                            }
                        }
                        if press {
                            if mods.logo {
                                match sym {
                                    keysyms::KEY_d if state.desktop.panel.is_some() => {
                                        state.toggle_desktop();
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_l if state.desktop.panel.is_some() => {
                                        state.begin_lock();
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_space if state.desktop.panel.is_some() => {
                                        state.shell_message(&serde_json::json!({"open":"start"}));
                                        return FilterResult::Intercept(());
                                    }
                                    n if (keysyms::KEY_1..=keysyms::KEY_4).contains(&n) => {
                                        state.switch_workspace((n - keysyms::KEY_1 + 1) as u8);
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_Tab => {
                                        if let Some(w) = state
                                            .minimized
                                            .iter()
                                            .rev()
                                            .find(|w| {
                                                state.desktop.tasks.get(w).is_some_and(|t| {
                                                    t.workspace == state.desktop.workspace
                                                })
                                            })
                                            .cloned()
                                        {
                                            state.restore_window(&w);
                                        }
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_Return => {
                                        state.spawn_terminal();
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_q => {
                                        state.close_focused();
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_w => {
                                        state.reload_wallpaper();
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_comma => {
                                        state.spawn_settings();
                                        return FilterResult::Intercept(());
                                    }
                                    keysyms::KEY_Q => {
                                        state.request_logout();
                                        return FilterResult::Intercept(());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        FilterResult::Forward
                    },
                );
                if matches!(filtered,FilterResult::Forward) {
                    if announced {
                        self.defer_reader_key(event.key_code(),event.state(),serial,time,mods_changed);
                    }else{
                        keyboard.input_forward(self,event.key_code(),event.state(),serial,time,mods_changed);
                    }
                }
            }

            // Winit reports absolute pointer positions.
            InputEvent::PointerMotionAbsolute { event } => {
                let pos = {
                    let bounds = self.desktop_bounds();
                    self.clamp_pointer(
                        bounds.loc.to_f64() + event.position_transformed(bounds.size),
                    )
                };
                self.cursor_location = pos;
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                self.pointer_moved(pos, serial, time);
            }

            // libinput reports relative motion; accumulate onto `cursor_location` (our own
            // pointer position; the seat's `current_location` freezes during interactive
            // drags) and clamp to the output.
            InputEvent::PointerMotion { event } => {
                let pos = self.clamp_pointer(self.cursor_location + event.delta());
                self.cursor_location = pos;
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                self.pointer_moved(pos, serial, time);
            }

            InputEvent::PointerButton { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                let button = event.button_code();
                let button_state = event.state();
                let pointer = self.seat.get_pointer().unwrap();
                let mods = self.seat.get_keyboard().unwrap().modifier_state();

                if button_state == ButtonState::Pressed && self.window_menu.is_some() {
                    let row = self
                        .window_menu
                        .as_ref()
                        .and_then(|m| m.row(self.cursor_location));
                    self.menu_action(if button == BTN_LEFT { row } else { None });
                    return;
                }
                if button_state == ButtonState::Pressed {
                    self.record_input(serial, pointer.current_focus());
                }
                let layer = self.hit_layer(self.cursor_location, true).or_else(|| {
                    if self.space.element_under(self.cursor_location).is_none() {
                        self.hit_layer(self.cursor_location, false)
                    } else {
                        None
                    }
                });
                if !self.desktop.lock.active && button_state == ButtonState::Pressed {
                    if let Some((_, _, l)) = &layer {
                        if l.can_receive_keyboard_focus() {
                            self.layer_focus(l.wl_surface());
                        }
                    }
                }
                if !self.desktop.lock.active
                    && self.hit_layer(self.cursor_location, true).is_none()
                    && button == BTN_LEFT
                    && button_state == ButtonState::Pressed
                {
                    if let Some((w, offset)) = self.decoration_at(self.cursor_location) {
                        self.decoration_click(w, offset);
                        return;
                    }
                }
                if !self.desktop.lock.active
                    && layer.is_none()
                    && button_state == ButtonState::Pressed
                    && !pointer.is_grabbed()
                {
                    let loc = pointer.current_location();
                    if let Some((window, win_loc)) =
                        self.space.element_under(loc).map(|(w, l)| (w.clone(), l))
                    {
                        crate::focus::set_keyboard_focus(self, Some(&window));
                        // Super OR Alt starts an interactive move. Alt is the practical
                        // trigger when running nested: a host compositor (e.g. Mutter)
                        // typically grabs Super+drag for itself, so Super never reaches us.
                        if button == BTN_LEFT && (mods.logo || mods.alt) && !window.toplevel().is_some_and(|t| t.current_state().states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized)) {
                            let offset = loc - self.space.element_location(&window).unwrap_or(win_loc).to_f64();
                            self.move_state = Some(MoveState {
                                window,
                                offset,
                                snap_preview: None,
                                client_grab: false,
                            });
                            return;
                        }
                    }
                }

                if button == BTN_LEFT && button_state == ButtonState::Released {
                    if let Some(mv) = self.move_state.take() {
                        let forward_release = mv.client_grab;
                        self.finish_move(mv);
                        if !forward_release {
                            return;
                        }
                    }
                    if let Some(r) = self.resize.take() {
                        if let Some(t) = r.window.toplevel() {
                            t.with_pending_state(|s| s.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Resizing));
                            t.send_pending_configure();
                        }
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        serial,
                        time,
                        button,
                        state: button_state,
                    },
                );
                pointer.frame(self);
            }

            InputEvent::PointerAxis { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let mut frame = AxisFrame::new(event.time_msec());
                if let Some(v) = event.amount(Axis::Vertical) {
                    frame = frame.value(Axis::Vertical, v);
                }
                if let Some(h) = event.amount(Axis::Horizontal) {
                    frame = frame.value(Axis::Horizontal, h);
                }
                pointer.axis(self, frame);
                pointer.frame(self);
            }

            _ => {}
        }
    }

    /// Apply a new pointer position: while dragging, move the grabbed window (pinning its
    /// spring to the cursor); otherwise forward motion to the focused surface.
    pub(crate) fn pointer_moved(&mut self, pos: Point<f64, Logical>, serial: Serial, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        if let Some(menu) = self.window_menu.as_mut() {
            menu.hover(pos);
            return;
        }
        if self.resize.is_some() {
            self.resize_to(pos);
            return;
        }
        if let Some(window) = self.move_state.as_ref().map(|m| m.window.clone()) {
            let offset = self.move_state.as_ref().unwrap().offset;
            let new = pos - offset;
            let (x, y) = (new.x.round() as i32, new.y.round() as i32);
            self.place_window(&window, (x, y).into());
            // Recompute snap preview: where the magnetic snap would put the window
            // if released right now.
            let size = self
                .space
                .element_geometry(&window)
                .map(|g| g.size)
                .or_else(|| self.wm.size_of(&window))
                .unwrap_or_else(|| (0, 0).into());
            let snapped = self.snap_target(&window, (x, y).into(), size);
            let preview = if snapped.x != x || snapped.y != y {
                Some(smithay::utils::Rectangle::new(snapped, size))
            } else {
                None
            };
            if let Some(mv) = self.move_state.as_mut() {
                mv.snap_preview = preview;
            }
        } else {
            let focus = if self.desktop.lock.active {
                self.lock_pointer(pos)
            } else {
                self.hit_layer(pos, true)
                    .map(|(s, p, _)| (s, p))
                    .or_else(|| {
                        self.space
                            .element_under(pos)
                            .filter(|_| self.decoration_at(pos).is_none())
                            .and_then(|(w, loc)| {
                                w.surface_under(
                                    pos - loc.to_f64(),
                                    smithay::desktop::WindowSurfaceType::ALL,
                                )
                                .map(|(s, offset)| (s, (loc + offset).to_f64()))
                            })
                    })
                    .or_else(|| self.hit_layer(pos, false).map(|(s, p, _)| (s, p)))
            };
            pointer.motion(
                self,
                focus,
                &MotionEvent {
                    location: pos,
                    serial,
                    time,
                },
            );
            pointer.frame(self);
        }
    }

    /// Compute the magnetic snap target for a dropped window and aim its spring there.
    fn finish_move(&mut self, mv: MoveState) {
        let window = mv.window;
        let Some(desired) = self.space.element_location(&window) else {
            return;
        };
        let size = self
            .space
            .element_geometry(&window)
            .map(|g| g.size)
            .or_else(|| self.wm.size_of(&window))
            .unwrap_or_else(|| (0, 0).into());
        let target = self.snap_target(&window, desired, size);
        if self.config.reduced_motion || self.desktop.reduce_motion {
            self.place_window(&window, target);
            return;
        }

        if let Some(id) = self.wm.spring_id(&window) {
            if let Some(h) = self.animation.spring_mut(id) {
                h.set_target(target.x, target.y);
                // Zero velocity explicitly: pointer_moved pins it during the drag, but
                // a flick on release can leave a spike on the last motion. Start from
                // rest at the drop position so the spring's only motion is toward the
                // snap target.
                h.x.velocity = 0.0;
                h.y.velocity = 0.0;
            }
        }
        // Snap preview is cleared because `move_state` was taken before this call.
        tracing::info!(
            "released window at ({}, {}) -> magnetic snap to ({}, {})",
            desired.x,
            desired.y,
            target.x,
            target.y
        );
    }

    /// Close the currently keyboard-focused toplevel.
    pub fn close_focused(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if let Some(surface) = keyboard.current_focus() {
            if let Some(x) = &surface.x11 {
                let _ = x.close();
                return;
            }
            if let Some(window) = self.wm.window_for_surface(&surface.surface) {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                }
            }
        }
    }

    /// Spawn a terminal against this compositor's Wayland socket.
    ///
    /// Honours `$MUNDANE_TERMINAL` if set; otherwise tries a built-in list (GNOME
    /// Console `kgx` / `gnome-terminal` first) and launches the first one that starts.
    pub fn spawn_terminal(&self) {
        const DEFAULTS: &[&str] = &[
            "kgx",
            "gnome-terminal",
            "weston-terminal",
            "foot",
            "alacritty",
        ];
        let candidates: Vec<&str> = match self.spawn_command.as_deref() {
            Some(cmd) => vec![cmd],
            None => DEFAULTS.to_vec(),
        };
        for cmd in &candidates {
            match std::process::Command::new(cmd)
                .env("WAYLAND_DISPLAY", &self.socket_name)
                .spawn()
            {
                Ok(_) => {
                    tracing::info!("spawned terminal '{cmd}'");
                    return;
                }
                Err(e) => tracing::debug!("could not spawn '{cmd}': {e}"),
            }
        }
        tracing::warn!("no terminal found (tried: {})", candidates.join(", "));
    }

    /// Spawn the mundaneDE settings panel as a Wayland client.
    ///
    /// Honours `$MUNDANE_SETTINGS` if set; otherwise calls `mundane-settings` on PATH.
    pub fn spawn_settings(&self) {
        let cmd = self
            .spawn_settings_command
            .as_deref()
            .unwrap_or("mundane-settings");
        match std::process::Command::new(cmd)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .spawn()
        {
            Ok(_) => tracing::info!("spawned settings '{cmd}'"),
            Err(e) => tracing::warn!("could not spawn '{cmd}': {e}"),
        }
    }
}
