//! Multi-connector DRM/KMS on the active GPU. Each CRTC owns its scanout queue.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Format as DrmFormat, Fourcc};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu};
use smithay::output::{Mode as WlMode, PhysicalProperties};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::drm::control::ModeTypeFlags;
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{DeviceFd, Transform};
use smithay::wayland::socket::ListeningSocketSource;

use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};

use crate::config::MundaneConfig;
use crate::outputs::Head;
use crate::state::{ClientState, State};
use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::reexports::drm::control::{connector, crtc};
use std::collections::HashMap;

type Compositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    OutputPresentationFeedback,
    DrmDeviceFd,
>;

struct Scanout {
    head: Head,
    connector: connector::Info,
    compositor: Compositor,
    pending: bool,
    generation: u64,
    next_frame: Instant,
    submitted_generation: u64,
}

struct DrmBackend {
    session: LibSeatSession,
    drm: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    renderer: GlesRenderer,
    scanner: DrmScanner,
    heads: HashMap<crtc::Handle, Scanout>,
    last_frame: Instant,
    active: bool,
    sleeping: bool,
    trial: Option<Trial>,
}

struct Trial {
    key: crtc::Handle,
    previous: (
        smithay::reexports::drm::control::Mode,
        smithay::output::Scale,
        Transform,
    ),
    requested: crate::config::DisplayConfig,
    deadline: Instant,
    previous_position: Option<[i32; 2]>,
    generation: u64,
}

pub fn run_drm(config: MundaneConfig, notify: Option<&std::path::Path>) -> anyhow::Result<()> {
    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().map_err(|e| anyhow::anyhow!("event loop init failed: {e}"))?;
    let display: Display<State> = Display::new()?;
    let dh = display.handle();

    // --- Session + primary GPU -------------------------------------------------------------
    let (mut session, session_notifier) =
        LibSeatSession::new().map_err(|e| anyhow::anyhow!("libseat session init failed: {e}"))?;
    let seat_name = session.seat();

    let gpu_path = primary_gpu(&seat_name)
        .ok()
        .flatten()
        .or_else(|| {
            all_gpus(&seat_name)
                .ok()
                .and_then(|gpus| gpus.into_iter().next())
        })
        .ok_or_else(|| anyhow::anyhow!("no DRM GPU found for seat '{seat_name}'"))?;
    tracing::info!("using GPU {}", gpu_path.display());

    // --- Open the DRM device + GBM ---------------------------------------------------------
    let fd = session
        .open(
            &gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", gpu_path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)
        .map_err(|e| anyhow::anyhow!("DrmDevice init failed: {e}"))?;
    let gbm = GbmDevice::new(fd).map_err(|e| anyhow::anyhow!("GbmDevice init failed: {e}"))?;

    // --- EGL renderer ----------------------------------------------------------------------
    // SAFETY: the gbm device outlives the EGL display (both are kept alive by `DrmBackend`).
    let egl_display =
        unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| anyhow::anyhow!("EGL display: {e}"))?;
    let egl_context =
        EGLContext::new(&egl_display).map_err(|e| anyhow::anyhow!("EGL context: {e}"))?;
    // SAFETY: the context is current to this thread and not shared elsewhere.
    let renderer = unsafe { GlesRenderer::new(egl_context) }
        .map_err(|e| anyhow::anyhow!("GLES renderer: {e}"))?;

    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    let mut state = State::new(
        &dh,
        config,
        seat_name.clone(),
        event_loop.get_signal(),
        event_loop.handle(),
        socket_name.clone(),
    )?;
    state.advertise_dmabuf(
        &renderer,
        smithay::backend::drm::DrmNode::from_dev_id(drm.device_id()).ok(),
    );
    let _x11_listener = crate::x11::listen(&state)?;
    let mut backend = DrmBackend {
        session,
        drm,
        gbm,
        renderer,
        scanner: DrmScanner::new(),
        heads: HashMap::new(),
        last_frame: Instant::now(),
        active: true,
        sleeping: false,
        trial: None,
    };
    let monitor = UdevBackend::new(&seat_name)?;
    backend.rescan(&mut state)?;
    let backend = Rc::new(RefCell::new(backend));

    let vt = backend.clone();
    state.switch_vt = Some(Box::new(move |number| {
        if let Err(e) = vt.borrow_mut().session.change_vt(number) {
            tracing::warn!(?e, number, "switch VT");
        }
    }));

    let control = backend.clone();
    let _control_socket = super::output_control::listen(&state, move |state, command| {
        control.borrow_mut().control(state, command)
    })?;

    // --- libinput --------------------------------------------------------------------------
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        backend.borrow().session.clone().into(),
    );
    libinput_context
        .udev_assign_seat(&seat_name)
        .map_err(|()| anyhow::anyhow!("failed to assign libinput seat '{seat_name}'"))?;
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    // --- Event sources ---------------------------------------------------------------------
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
        .insert_source(libinput_backend, |event, _, state: &mut State| {
            state.handle_input(event);
        })
        .map_err(|e| anyhow::anyhow!("failed to insert libinput source: {e}"))?;

    let gpu = backend.borrow().drm.device_id();
    let devices = backend.clone();
    handle
        .insert_source(monitor, move |event, _, state| {
            if matches!(event,UdevEvent::Changed{device_id} if device_id==gpu) {
                if let Err(err) = devices.borrow_mut().rescan(state) {
                    tracing::error!(%err,"connector rescan failed");
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("register udev: {e}"))?;
    let flips = backend.clone();
    handle.insert_source(drm_notifier, move |event, metadata, state| {
        use smithay::backend::drm::DrmEventTime;
        use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
        use smithay::wayland::presentation::Refresh;
        match event {
            DrmEvent::VBlank(crtc) => {
                let mut backend = flips.borrow_mut();
                let Some(head) = backend.heads.get_mut(&crtc) else { return };
                if !head.pending { return; }
                head.pending = false;
                head.next_frame = Instant::now();
                match head.compositor.frame_submitted() {
                    Ok(Some(mut feedback)) => {
                        let mut time = state.clock.now();
                        let mut flags = Kind::Vsync;
                        let sequence = metadata.as_ref().map_or(0, |m| u64::from(m.sequence));
                        if let Some(DrmEventTime::Monotonic(stamp)) = metadata.as_ref().map(|m| m.time) {
                            time = stamp.into();
                            flags |= Kind::HwClock | Kind::HwCompletion;
                            tracing::debug!(target: "mundane_present", output = head.head.output.name(),
                                seq = sequence, ns = stamp.as_nanos() as u64, "DRM page flip");
                        }
                        head.submitted_generation = head.generation;
                        state.lock_frame(&head.head.output,head.generation);
                        feedback.presented(time, Refresh::fixed(head.period()), sequence, flags);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!(%e, "complete DRM frame"),
                }
            }
            DrmEvent::Error(e) => tracing::error!(%e, "DRM event error"),
        }
    }).map_err(|e| anyhow::anyhow!("register DRM events: {e}"))?;
    let seat_backend = backend.clone();
    handle
        .insert_source(session_notifier, move |event, _, state| {
            let mut b = seat_backend.borrow_mut();
            match event {
                SessionEvent::PauseSession => {
                    tracing::info!("seat paused");
                    state.desktop.pending_keys.clear();
                    b.active = false;
                    libinput_context.suspend();
                    b.drm.pause();
                }
                SessionEvent::ActivateSession => {
                    if let Err(e) = b.drm.activate(false) {
                        tracing::error!(%e,"resume DRM");
                        return;
                    }
                    tracing::info!("seat resumed");
                    b.active = true;
                    for head in b.heads.values_mut() {
                        head.pending = false;
                        head.compositor.reset_buffers();
                    }
                    let _ = b.rescan(state);
                    state.generation += 1;
                    if let Err(e) = libinput_context.resume() {
                        tracing::warn!(?e, "resume input");
                    }
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("register seat: {e}"))?;

    // Hot-reload watcher (best-effort).
    let _hot_reload = match crate::hot_reload::install(&handle) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("hot-reload disabled: {e}");
            None
        }
    };

    if let Some(endpoint) = notify {
        let devices = backend.clone();
        super::session::ready(&mut state, endpoint, move |state, sleeping| {
            devices.borrow_mut().sleep_changed(state, sleeping);
        })?;
    }
    state.launch_helpers()?;
    tracing::info!("mundaneDE DRM backend running on WAYLAND_DISPLAY={socket_name}");

    let mut wake = crate::outputs::FrameWake::default();
    let result = event_loop
        .run(None, &mut state, |state| {
            backend.borrow_mut().render(state, &mut wake);
            state.popups.cleanup();
            state.space.refresh();
            let _ = state.display_handle.flush_clients();
        })
        .map_err(|e| anyhow::anyhow!("event loop: {e}"));
    if notify.is_some() {
        // GDM takes over from here. Release the scanouts while logind still
        // grants us the seat, before dropping the event sources in the loop.
        let mut devices = backend.borrow_mut();
        devices.heads.clear();
        devices.drm.pause();
    }
    result
}

impl Scanout {
    fn period(&self) -> Duration {
        Duration::from_secs_f64(
            1000. / self.head.output.current_mode().unwrap().refresh.max(1000) as f64,
        )
    }
}

impl DrmBackend {
    fn control(
        &mut self,
        state: &mut State,
        cmd: super::output_control::Command,
    ) -> anyhow::Result<String> {
        use super::output_control::{save, Command};
        if !matches!(cmd,Command::Revert) {
            anyhow::ensure!(self.active && !state.desktop.lock.active,"session is inactive or locked");
        }
        match cmd {
            Command::List => {
                let mode = |m:WlMode| serde_json::json!({"width":m.size.w,"height":m.size.h,"refresh_hz": m.refresh as f64 / 1000.});
                let mut outputs=Vec::new();
                for h in self.heads.values(){
                    let o=&h.head.output;
                    let rotation=[0,90,180,270].into_iter().find(|deg|crate::outputs::rotation(*deg)==Some(o.current_transform())).unwrap_or(0);
                    outputs.push(serde_json::json!({"name":o.name(),"current":o.current_mode().map(mode),"scale":o.current_scale().fractional_scale(),"rotation":rotation,"position":crate::outputs::position(o),"origin":[o.current_location().x,o.current_location().y],"connected":true,"primary":false,"modes":h.connector.modes().iter().map(|m|mode((*m).into())).collect::<Vec<_>>()}));
                }
                outputs.sort_by(|a,b|a["name"].as_str().cmp(&b["name"].as_str()));
                return Ok(serde_json::to_string(&outputs)?);
            }
            Command::Test {
                name,
                width,
                height,
                refresh,
                scale,
                rotation,
                position,
            } => {
                anyhow::ensure!(
                    self.active && self.trial.is_none(),
                    "another display trial is active or the seat is paused"
                );
                anyhow::ensure!(
                    crate::outputs::valid_scale(scale),
                    "scale must be between 0.5 and 4"
                );
                let transform = crate::outputs::rotation(rotation)
                    .ok_or_else(|| anyhow::anyhow!("unsupported rotation"))?;
                let (key, h) = self
                    .heads
                    .iter_mut()
                    .find(|(_, h)| h.head.output.name() == name)
                    .ok_or_else(|| anyhow::anyhow!("unknown connector"))?;
                let mode = h
                    .connector
                    .modes()
                    .iter()
                    .copied()
                    .find(|m| {
                        let wl = WlMode::from(*m);
                        [wl.size.w, wl.size.h, wl.refresh] == [width, height, refresh]
                    })
                    .ok_or_else(|| anyhow::anyhow!("mode is not advertised"))?;
                let o = &h.head.output;
                let previous_position = crate::outputs::position(o);
                let position = match position.as_slice() {
                    [] => previous_position,
                    &[x, y] if crate::outputs::valid_position([x, y]) => Some([x, y]),
                    _ => anyhow::bail!("invalid display position"),
                };
                let previous = (
                    h.compositor.pending_mode(),
                    o.current_scale(),
                    o.current_transform(),
                );
                h.compositor.use_mode(mode)?;
                h.compositor.reset_buffers();
                crate::outputs::set_position(o, position);
                o.change_current_state(
                    Some(mode.into()),
                    Some(transform),
                    Some(smithay::output::Scale::Fractional(scale)),
                    None,
                );
                self.trial = Some(Trial {
                    key: *key,
                    previous,
                    previous_position,
                    generation: state.generation + 1,
                    requested: crate::config::DisplayConfig {
                        position,
                        mode: Some([width, height, refresh]),
                        scale: Some(scale),
                        rotation: Some(rotation),
                    },
                    deadline: Instant::now() + Duration::from_secs(15),
                });
            }
            Command::Confirm => {
                let t = self
                    .trial
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no display trial"))?;
                anyhow::ensure!(Instant::now() < t.deadline, "display trial expired");
                anyhow::ensure!(
                    self.heads[&t.key].submitted_generation >= t.generation,
                    "waiting for the trial page flip"
                );
                save(&self.heads[&t.key].head.output.name(), t.requested.clone())?;
                self.trial = None;
            }
            Command::Revert => {
                let t = self
                    .trial
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no display trial"))?;
                let h = self.heads.get_mut(&t.key).unwrap();
                let (mode, scale, transform) = t.previous;
                h.compositor.use_mode(mode)?;
                h.compositor.reset_buffers();
                crate::outputs::set_position(&h.head.output, t.previous_position);
                h.head.output.change_current_state(
                    Some(mode.into()),
                    Some(transform),
                    Some(scale),
                    None,
                );
                self.trial = None;
            }
        }
        state.arrange_heads();
        Ok("ok".into())
    }
    fn rescan(&mut self, state: &mut State) -> anyhow::Result<()> {
        for event in self.scanner.scan_connectors(&self.drm)? {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    if let Err(e) = self.connect(state, connector, crtc) {
                        tracing::error!(%e,"connect output");
                    }
                }
                DrmScanEvent::Disconnected {
                    crtc: Some(crtc), ..
                } => {
                    if self.trial.as_ref().is_some_and(|t| t.key == crtc) {
                        self.trial = None;
                    }
                    if let Some(head) = self.heads.remove(&crtc) {
                        tracing::info!(output = head.head.output.name(), "output disconnected");
                        state.remove_head(head.head);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn connect(
        &mut self,
        state: &mut State,
        connector: connector::Info,
        crtc: crtc::Handle,
    ) -> anyhow::Result<()> {
        let name = format!(
            "{}-{}",
            connector.interface().as_str(),
            connector.interface_id()
        );
        let saved = super::output_control::load().unwrap_or_else(|e| {
            tracing::warn!(%e,"read confirmed outputs");
            Default::default()
        });
        let selected = saved
            .get(&name)
            .or_else(|| state.config.displays.get(&name))
            .cloned();
        let pref = selected.as_ref();
        let modes = connector.modes();
        let preferred = modes
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or(modes.first())
            .ok_or_else(|| anyhow::anyhow!("connector has no modes"))?;
        let mode = pref
            .and_then(|p| p.mode)
            .and_then(|v| {
                modes.iter().find(|m| {
                    let wl = WlMode::from(**m);
                    [wl.size.w, wl.size.h, wl.refresh] == v
                })
            })
            .unwrap_or(preferred);
        let transform = crate::outputs::rotation(pref.and_then(|p| p.rotation).unwrap_or(0))
            .unwrap_or(Transform::Normal);
        let surface = self
            .drm
            .create_surface(crtc, *mode, &[connector.handle()])?;
        let (w, h) = connector.size().unwrap_or_default();
        let head = state.add_head(
            name.clone(),
            PhysicalProperties {
                size: (w as i32, h as i32).into(),
                subpixel: connector.subpixel().into(),
                make: "Unknown".into(),
                model: name.clone(),
            },
            (*mode).into(),
            transform,
        );
        let formats: Vec<DrmFormat> = self
            .renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect();
        let comp = DrmCompositor::new(
            &head.output,
            surface,
            None,
            GbmAllocator::new(
                self.gbm.clone(),
                GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
            ),
            GbmFramebufferExporter::new(self.gbm.clone(), None),
            [Fourcc::Argb8888, Fourcc::Abgr8888],
            formats,
            self.drm.cursor_size(),
            Some(self.gbm.clone()),
        );
        let compositor = match comp {
            Ok(c) => c,
            Err(e) => {
                state.remove_head(head);
                anyhow::bail!("initialize scanout: {e}");
            }
        };
        crate::outputs::set_position(
            &head.output,
            pref.and_then(|p| p.position)
                .filter(|p| crate::outputs::valid_position(*p)),
        );
        if let Some(scale) = pref
            .and_then(|c| c.scale)
            .filter(|s| crate::outputs::valid_scale(*s))
        {
            head.output.change_current_state(
                None,
                None,
                Some(smithay::output::Scale::Fractional(scale)),
                None,
            );
            state.arrange_heads();
        }
        for m in modes {
            head.output.add_mode((*m).into());
        }
        state.arrange_heads();
        head.output.set_preferred((*preferred).into());
        tracing::info!(%name,?mode,"output connected");
        self.heads.insert(
            crtc,
            Scanout {
                head,
                connector,
                compositor,
                pending: false,
                generation: 0,
                submitted_generation: 0,
                next_frame: Instant::now(),
            },
        );
        Ok(())
    }
    fn sleep_changed(&mut self, state: &mut State, sleeping: bool) {
        self.sleeping = sleeping;
        if !self.active {
            return;
        }
        for head in self.heads.values_mut() {
            // Stop scanning out before logind releases the sleep inhibitor. S3
            // can invalidate the device-side resources behind the old GBM BOs.
            let result = if sleeping {
                head.compositor.clear()
            } else {
                head.compositor.reset_state()
            };
            if let Err(e) = result {
                tracing::warn!(%e, sleeping, "reset scanout across sleep");
            }
            head.compositor.reset_buffers();
            head.pending = false;
            head.next_frame = Instant::now();
        }
        if !sleeping {
            if let Err(e) = self.rescan(state) {
                tracing::warn!(%e, "rescan after sleep");
            }
            self.last_frame = Instant::now();
        }
        state.generation += 1;
        tracing::info!(sleeping, "sleep transition");
    }

    fn render(&mut self, state: &mut State, wake: &mut crate::outputs::FrameWake) {
        if !self.active || self.sleeping {
            return;
        }
        state.import_buffers(&mut self.renderer);
        state.copy_capture_frames(&mut self.renderer);
        if let Some(t) = &self.trial {
            if Instant::now() >= t.deadline {
                if let Err(e) = self.control(state, super::output_control::Command::Revert) {
                    tracing::error!(%e,"revert display trial");
                }
            } else {
                wake.schedule(&state.handle, t.deadline);
            }
        }
        let now = Instant::now();
        let animating = state.animation.has_active_animations();
        let due = self.heads.values().any(|h| {
            !h.pending && now >= h.next_frame && (animating || h.generation != state.generation)
        });
        if due {
            if animating {
                state.animate(now - self.last_frame);
                state.generation += 1;
            }
            self.last_frame = now;
            state.space.refresh();
            state.update_scales();
        }
        for h in self.heads.values_mut() {
            if h.pending || h.generation == state.generation && !animating {
                continue;
            }
            if now < h.next_frame {
                wake.schedule(&state.handle, h.next_frame);
                continue;
            }
            let period = h.period();
            let o = &h.head.output;
            let (elements, clear) =
                crate::render::collect_elements(&mut self.renderer, state, o, true);
            match h.compositor.render_frame(
                &mut self.renderer,
                &elements,
                clear,
                FrameFlags::DEFAULT,
            ) {
                Ok(frame) => {
                    if frame.needs_sync() {
                        if let smithay::backend::drm::compositor::PrimaryPlaneElement::Swapchain(
                            primary,
                        ) = &frame.primary_element
                        {
                            if let Err(e) = primary.sync.wait() {
                                tracing::warn!(?e, "wait for scanout buffer");
                                h.next_frame = now + period;
                                wake.schedule(&state.handle, h.next_frame);
                                continue;
                            }
                        }
                    }
                    let feedback = (!frame.is_empty).then(|| state.feedback(o, &frame.states));
                    drop(frame);
                    if let Some(feedback) = feedback {
                        match h.compositor.queue_frame(feedback) {
                            Ok(()) => {
                                h.pending = true;
                                h.generation = state.generation;
                            }
                            Err(e) => tracing::warn!(%e,"queue DRM frame"),
                        }
                    } else {
                        h.generation = state.generation;
                    }
                }
                Err(e) => tracing::warn!(%e,"render DRM frame"),
            }
            state.send_frames(o);
            h.next_frame = now + h.period();
            if !h.pending
                && (h.generation != state.generation || state.animation.has_active_animations())
            {
                wake.schedule(&state.handle, h.next_frame);
            }
        }
    }
}
