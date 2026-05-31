//! The DRM/KMS production backend (single output).
//!
//! Drives real hardware: a [`LibSeatSession`] for the seat/VT, udev to find the primary GPU,
//! a GBM allocator, an EGL [`GlesRenderer`], and libinput for input. Rendering goes through a
//! [`DrmCompositor`], which owns its own damage tracker and page-flip queue. Because it needs
//! a free TTY/seat it can only run on real hardware or in the bundled QEMU (see
//! `tests/qemu/`); for everyday development use `--winit`.
//!
//! This is deliberately a *single-output* backend: it picks the first connected connector on
//! the primary GPU and drives that one CRTC. Multi-output and hotplug are future work.
//!
//! ## Frame scheduling
//!
//! The calloop loop runs with a ~16 ms timeout, so the main callback fires roughly every
//! frame. Each tick, unless a page flip is still in flight (`pending_flip`), we advance the
//! springs and ask the compositor to render. If anything was drawn we queue the flip and set
//! `pending_flip`; the DRM VBlank event then calls `frame_submitted` and clears the flag,
//! letting the next tick render again. When nothing changed `render_frame` reports empty and
//! we simply try again next tick — cheap, and it naturally picks up new client buffers and
//! ongoing animations without a separate repaint timer.

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
use crate::state::{BackendIdentity, ClientState, State};

/// The concrete compositor type for our single GBM-backed GLES output.
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// Backend objects that live outside [`State`] (which stays backend-agnostic) and are shared
/// between the loop callback and the DRM/session event sources via `Rc<RefCell<…>>`.
struct DrmBackend {
    // Kept alive for the lifetime of the backend (own the device fd / seat connection).
    #[allow(dead_code)]
    session: LibSeatSession,
    #[allow(dead_code)]
    drm: DrmDevice,
    #[allow(dead_code)]
    gbm: GbmDevice<DrmDeviceFd>,

    renderer: GlesRenderer,
    compositor: Compositor,
    last_frame: Instant,
    /// True while a queued page flip is awaiting its VBlank; suppresses rendering until then.
    pending_flip: bool,
}

pub fn run_drm(config: MundaneConfig) -> anyhow::Result<()> {
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

    let (mut drm, drm_notifier) = DrmDevice::new(fd.clone(), true)
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

    // --- Pick the first connected connector + its CRTC and preferred mode ------------------
    let mut scanner: DrmScanner = DrmScanner::new();
    let scan = scanner
        .scan_connectors(&drm)
        .map_err(|e| anyhow::anyhow!("connector scan failed: {e}"))?;
    let (connector, crtc) = scan
        .into_iter()
        .find_map(|event| match event {
            DrmScanEvent::Connected {
                connector,
                crtc: Some(crtc),
            } => Some((connector, crtc)),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no connected output found"))?;

    let output_name = format!(
        "{}-{}",
        connector.interface().as_str(),
        connector.interface_id()
    );
    let mode_id = connector
        .modes()
        .iter()
        .position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .unwrap_or(0);
    let drm_mode = connector.modes()[mode_id];
    let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
    let wl_mode = WlMode::from(drm_mode);
    tracing::info!(
        "output {output_name}: {}x{} @ {} mHz",
        wl_mode.size.w,
        wl_mode.size.h,
        wl_mode.refresh
    );

    // --- Wayland socket + compositor State -------------------------------------------------
    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let loop_signal = event_loop.get_signal();
    let identity = BackendIdentity {
        seat_name: seat_name.clone(),
        output_name: output_name.clone(),
        mode: wl_mode,
        transform: Transform::Normal,
        physical: PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: connector.subpixel().into(),
            make: "Unknown".to_string(),
            model: output_name.clone(),
        },
    };
    let mut state = State::new(&dh, config, identity, loop_signal, socket_name.clone())?;

    // --- DRM surface + compositor ----------------------------------------------------------
    let cursor_size = drm.cursor_size();
    let surface = drm
        .create_surface(crtc, drm_mode, &[connector.handle()])
        .map_err(|e| anyhow::anyhow!("create_surface failed: {e}"))?;
    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let render_formats: Vec<DrmFormat> = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect();
    let compositor: Compositor = DrmCompositor::new(
        &state.output,
        surface,
        None,
        allocator,
        exporter,
        [Fourcc::Argb8888, Fourcc::Abgr8888],
        render_formats,
        cursor_size,
        Some(gbm.clone()),
    )
    .map_err(|e| anyhow::anyhow!("DrmCompositor init failed: {e}"))?;

    let backend = Rc::new(RefCell::new(DrmBackend {
        session,
        drm,
        gbm,
        renderer,
        compositor,
        last_frame: Instant::now(),
        pending_flip: false,
    }));

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
        .insert_source(libinput_backend, |event, _, state: &mut State| {
            state.handle_input(event);
        })
        .map_err(|e| anyhow::anyhow!("failed to insert libinput source: {e}"))?;

    {
        let backend = backend.clone();
        handle
            .insert_source(drm_notifier, move |event, _meta, _state: &mut State| {
                match event {
                    DrmEvent::VBlank(_crtc) => {
                        let mut b = backend.borrow_mut();
                        if let Err(e) = b.compositor.frame_submitted() {
                            tracing::error!("frame_submitted: {e}");
                        }
                        b.pending_flip = false;
                    }
                    DrmEvent::Error(e) => tracing::error!("DRM error: {e}"),
                }
            })
            .map_err(|e| anyhow::anyhow!("failed to insert drm source: {e}"))?;
    }

    handle
        .insert_source(
            session_notifier,
            move |event, _, _state: &mut State| match event {
                SessionEvent::PauseSession => {
                    tracing::info!("session paused");
                    libinput_context.suspend();
                }
                SessionEvent::ActivateSession => {
                    tracing::info!("session resumed");
                    if let Err(e) = libinput_context.resume() {
                        tracing::warn!("failed to resume libinput: {e:?}");
                    }
                }
            },
        )
        .map_err(|e| anyhow::anyhow!("failed to insert session source: {e}"))?;

    tracing::info!("mundaneDE DRM backend running on WAYLAND_DISPLAY={socket_name}");

    // --- Run -------------------------------------------------------------------------------
    event_loop
        .run(Some(Duration::from_millis(16)), &mut state, move |state| {
            render(&backend, state);
        })
        .map_err(|e| anyhow::anyhow!("event loop error: {e}"))?;

    Ok(())
}

/// Advance animations and present one frame, unless a page flip is still pending. Always sends
/// frame callbacks and flushes clients so they keep producing buffers.
fn render(backend: &Rc<RefCell<DrmBackend>>, state: &mut State) {
    {
        let mut guard = backend.borrow_mut();
        let b = &mut *guard;
        if !b.pending_flip {
            let now = Instant::now();
            let dt = now - b.last_frame;
            b.last_frame = now;

            if state.animation.tick(dt) {
                crate::render::sync_spring_positions(state);
            }
            state.wallpaper.cleanup(&mut state.animation);

            let (elements, clear_color) =
                crate::render::collect_elements(&mut b.renderer, state, true);
            match b.compositor.render_frame(
                &mut b.renderer,
                &elements,
                clear_color,
                FrameFlags::DEFAULT,
            ) {
                Ok(result) => {
                    if !result.is_empty {
                        match b.compositor.queue_frame(()) {
                            Ok(()) => b.pending_flip = true,
                            Err(e) => tracing::error!("queue_frame: {e}"),
                        }
                    }
                }
                Err(e) => tracing::error!("render_frame: {e}"),
            }
        }
    }

    let time = state.start_time.elapsed();
    let output = state.output.clone();
    for window in state.space.elements() {
        window.send_frame(&output, time, None, |_, _| Some(output.clone()));
    }
    state.space.refresh();
    let _ = state.display_handle.flush_clients();
}
