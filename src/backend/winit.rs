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
use smithay::reexports::wayland_server::Display;
use smithay::utils::{Logical, Point, Serial, Transform, SERIAL_COUNTER};
use smithay::wayland::socket::ListeningSocketSource;

use crate::config::MundaneConfig;
use crate::state::{BackendIdentity, ClientState, MoveState, State};

/// Left mouse button (from `linux/input-event-codes.h`).
const BTN_LEFT: u32 = 0x110;

pub fn run_winit(config: MundaneConfig) -> anyhow::Result<()> {
    // NOTE: calloop's `InsertError<Source>` and winit/xkb error types embed non-Send/Sync
    // internals (Rc, NonNull<xkb_*>, channel Receivers), so they can't convert to
    // `anyhow::Error` via `?`. Render each to a string-based error with `map_err` instead.
    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().map_err(|e| anyhow::anyhow!("event loop init failed: {e}"))?;
    let display: Display<State> = Display::new()?;
    let dh = display.handle();

    let (mut backend, winit) =
        winit::init::<GlesRenderer>().map_err(|e| anyhow::anyhow!("winit init failed: {e}"))?;

    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let loop_signal = event_loop.get_signal();
    let identity = BackendIdentity {
        seat_name: "winit".to_string(),
        output_name: "winit".to_string(),
        mode: Mode {
            size: (config.output.width, config.output.height).into(),
            refresh: config.output.refresh,
        },
        // The winit host window presents upside-down without this flip.
        transform: Transform::Flipped180,
        physical: PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "mundaneDE".to_string(),
            model: "Winit".to_string(),
        },
    };
    let mut state = State::new(&dh, config, identity, loop_signal, socket_name.clone())?;

    let handle = event_loop.handle();

    handle
        .insert_source(socket, move |stream, _, state: &mut State| {
            if let Err(e) = state
                .display_handle
                .insert_client(stream, Arc::new(ClientState::default()))
            {
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
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("failed to insert display source: {e}"))?;

    handle
        .insert_source(winit, move |event, _, state: &mut State| match event {
            WinitEvent::Resized { size, .. } => {
                let refresh = state.config.output.refresh;
                state
                    .output
                    .change_current_state(Some(Mode { size, refresh }), None, None, None);
            }
            WinitEvent::Input(ev) => state.handle_input(ev),
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            WinitEvent::Focus(_) | WinitEvent::Redraw => {}
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
    event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        let now = Instant::now();
        let dt = now - last_frame;
        last_frame = now;

        if state.animation.tick(dt) {
            crate::render::sync_spring_positions(state);
        }
        state.wallpaper.cleanup(&mut state.animation);

        if let Err(e) = render(&mut backend, state) {
            tracing::error!("render error: {e}");
        }

        let time = state.start_time.elapsed();
        let output = state.output.clone();
        for window in state.space.elements() {
            window.send_frame(&output, time, None, |_, _| Some(output.clone()));
        }
        state.space.refresh();
        let _ = state.display_handle.flush_clients();
    })
    .map_err(|e| anyhow::anyhow!("event loop error: {e}"))?;

    Ok(())
}

fn render(backend: &mut WinitGraphicsBackend<GlesRenderer>, state: &mut State) -> anyhow::Result<()> {
    let age = backend.buffer_age().unwrap_or(0);
    let damage = {
        let (renderer, mut framebuffer) =
            backend.bind().map_err(|e| anyhow::anyhow!("bind: {e}"))?;
        // winit runs nested, so leave the cursor to the host compositor.
        let (elements, clear_color) = crate::render::collect_elements(renderer, state, false);
        let res = state
            .damage_tracker
            .render_output(renderer, &mut framebuffer, age, &elements, clear_color)
            .map_err(|e| anyhow::anyhow!("render_output: {e:?}"))?;
        res.damage.cloned()
    };

    if let Some(damage) = damage {
        backend
            .submit(Some(&damage))
            .map_err(|e| anyhow::anyhow!("submit: {e}"))?;
    }
    Ok(())
}

impl State {
    /// Dispatch an input event from any backend: keybindings, click-to-focus, and interactive
    /// move. Generic over [`InputBackend`] so the winit (absolute pointer) and DRM (libinput,
    /// relative pointer) backends share one handler.
    pub fn handle_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => {
                let keyboard = self.seat.get_keyboard().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                let press = event.state() == KeyState::Pressed;
                keyboard.input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |state, mods, handle| {
                        let sym = handle.modified_sym().raw();
                        if press {
                            if sym == keysyms::KEY_Escape {
                                state.loop_signal.stop();
                                return FilterResult::Intercept(());
                            }
                            if mods.logo {
                                match sym {
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
                                        state.loop_signal.stop();
                                        return FilterResult::Intercept(());
                                    }
                                    _ => {}
                                }
                            }
                        }
                        FilterResult::Forward
                    },
                );
            }

            // Winit reports absolute pointer positions.
            InputEvent::PointerMotionAbsolute { event } => {
                let pos = event.position_transformed(self.output_logical_size());
                self.cursor_location = pos;
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                self.pointer_moved(pos, serial, time);
            }

            // libinput reports relative motion; accumulate onto `cursor_location` (our own
            // pointer position; the seat's `current_location` freezes during interactive
            // drags) and clamp to the output.
            InputEvent::PointerMotion { event } => {
                let size = self.output_logical_size();
                let next = self.cursor_location + event.delta();
                let pos = Point::from((
                    next.x.clamp(0.0, size.w as f64),
                    next.y.clamp(0.0, size.h as f64),
                ));
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

                if button == BTN_LEFT && button_state == ButtonState::Pressed {
                    let loc = pointer.current_location();
                    if let Some((window, win_loc)) =
                        self.space.element_under(loc).map(|(w, l)| (w.clone(), l))
                    {
                        crate::focus::set_keyboard_focus(self, Some(&window));
                        // Super OR Alt starts an interactive move. Alt is the practical
                        // trigger when running nested: a host compositor (e.g. Mutter)
                        // typically grabs Super+drag for itself, so Super never reaches us.
                        if mods.logo || mods.alt {
                            let offset = loc - win_loc.to_f64();
                            self.move_state = Some(MoveState {
                                window,
                                offset,
                                snap_preview: None,
                            });
                            return;
                        }
                    }
                }

                if button == BTN_LEFT && button_state == ButtonState::Released {
                    if let Some(mv) = self.move_state.take() {
                        self.finish_move(mv);
                        pointer.frame(self);
                        return;
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
    fn pointer_moved(&mut self, pos: Point<f64, Logical>, serial: Serial, time: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        if let Some(window) = self.move_state.as_ref().map(|m| m.window.clone()) {
            let offset = self.move_state.as_ref().unwrap().offset;
            let new = pos - offset;
            let (x, y) = (new.x.round() as i32, new.y.round() as i32);
            self.space.map_element(window.clone(), (x, y), false);
            // Pin the spring to the pointer so release springs from rest here.
            if let Some(id) = self.wm.spring_id(&window) {
                if let Some(h) = self.animation.spring_mut(id) {
                    h.x.position = x as f64;
                    h.x.target = x as f64;
                    h.x.velocity = 0.0;
                    h.y.position = y as f64;
                    h.y.target = y as f64;
                    h.y.velocity = 0.0;
                }
            }
            // Recompute snap preview: where the magnetic snap would put the window
            // if released right now.
            let size = self
                .space
                .element_geometry(&window)
                .map(|g| g.size)
                .or_else(|| self.wm.size_of(&window))
                .unwrap_or_else(|| (0, 0).into());
            let output = self.output_logical_size();
            let neighbors = self.wm.neighbors_except(&window, &self.space);
            let snapped = crate::wm::snap(
                Point::from((x, y)),
                size,
                output,
                &neighbors,
                self.config.wm.attraction_radius,
                self.config.wm.outer_gap,
                self.config.wm.inner_gap,
            );
            let preview = if snapped.x != x || snapped.y != y {
                Some(smithay::utils::Rectangle::new(snapped, size))
            } else {
                None
            };
            if let Some(mv) = self.move_state.as_mut() {
                mv.snap_preview = preview;
            }
        } else {
            let focus = self
                .space
                .element_under(pos)
                .and_then(|(w, loc)| w.toplevel().map(|t| (t.wl_surface().clone(), loc.to_f64())));
            pointer.motion(self, focus, &MotionEvent { location: pos, serial, time });
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
        let output_size = self.output_logical_size();
        let neighbors = self.wm.neighbors_except(&window, &self.space);
        let target = crate::wm::snap(
            desired,
            size,
            output_size,
            &neighbors,
            self.config.wm.attraction_radius,
            self.config.wm.outer_gap,
            self.config.wm.inner_gap,
        );
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
            if let Some(window) = self.wm.window_for_surface(&surface) {
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
        const DEFAULTS: &[&str] = &["kgx", "gnome-terminal", "weston-terminal", "foot", "alacritty"];
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
