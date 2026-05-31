//! The compositor state and its Wayland protocol handler implementations.
//!
//! [`State`] owns every piece of Wayland protocol state, the desktop [`Space`], the
//! animation engine and the magnetic window manager. The handler traits below wire the
//! Smithay-managed protocols (compositor, shm, xdg-shell, seat, data-device, output) to
//! this state; the `delegate_*!` macros generate the dispatch glue.

use std::os::unix::io::OwnedFd;
use std::time::Instant;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::desktop::{Space, Window};
use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties};
use smithay::reexports::calloop::LoopSignal;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle};
use smithay::utils::{Logical, Point, Transform};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
};

use crate::animation::AnimationEngine;
use crate::config::MundaneConfig;
use crate::wallpaper::WallpaperManager;
use crate::wm::WindowManager;

/// Backend-supplied identity for the seat and the single output.
///
/// `State::new` is backend-agnostic, so the winit and DRM backends each pass the names,
/// mode, transform and physical properties appropriate to their device. winit synthesises
/// these from `[output]` config; DRM derives them from the seat and the chosen connector.
pub struct BackendIdentity {
    pub seat_name: String,
    pub output_name: String,
    pub mode: Mode,
    pub transform: Transform,
    pub physical: PhysicalProperties,
}

/// An in-progress interactive window move (Super + left drag).
pub struct MoveState {
    /// The window being dragged.
    pub window: Window,
    /// Pointer-minus-window-origin captured at grab start, so the window tracks the
    /// pointer 1:1 instead of jumping its top-left to the cursor.
    pub offset: Point<f64, Logical>,
    /// Where the magnetic snap would place this window if released right now, when that
    /// position differs from the raw drop. `None` means "no snap candidate in range" —
    /// the render path uses this to draw a ghost rectangle showing the user where the
    /// window will land on release.
    pub snap_preview: Option<smithay::utils::Rectangle<i32, Logical>>,
}

/// Per-client state required by the compositor protocol.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

pub struct State {
    pub display_handle: DisplayHandle,

    // Wayland protocol globals.
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub seat: Seat<Self>,
    // Retained to keep the wl_output / xdg-output manager globals alive.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,

    // Desktop layout + output.
    pub space: Space<Window>,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    _output_global: GlobalId,

    // mundaneDE-specific.
    pub animation: AnimationEngine,
    pub wm: WindowManager,
    pub wallpaper: WallpaperManager,
    pub config: MundaneConfig,

    // The current cursor image requested by clients/seat. The built-in arrow is rebuilt
    // each frame in `render::cursor_elements` so the [`OutputDamageTracker`] sees a fresh
    // `Id` and correctly damages both the old and new positions on every move.
    pub cursor_status: CursorImageStatus,
    /// Authoritative pointer position. Updated for every motion event regardless of any
    /// in-progress interactive grab; the seat's `pointer.current_location()` can't serve as
    /// the source of truth because we deliberately skip `pointer.motion(...)` during a drag
    /// (otherwise the grabbed window would receive stray motion events), which freezes its
    /// reported location and breaks the libinput relative-motion accumulator (`current +
    /// delta`).
    pub cursor_location: Point<f64, Logical>,

    // Interactive move (manual grab — see [`MoveState`]).
    pub move_state: Option<MoveState>,

    // Loop control / timing.
    pub start_time: Instant,
    pub loop_signal: LoopSignal,
    pub socket_name: String,
    /// `$MUNDANE_TERMINAL` override; when `None`, a built-in candidate list is tried.
    pub spawn_command: Option<String>,
    /// `$MUNDANE_SETTINGS` override; when `None`, plain `mundane-settings` on PATH.
    pub spawn_settings_command: Option<String>,
}

impl State {
    pub fn new(
        dh: &DisplayHandle,
        config: MundaneConfig,
        identity: BackendIdentity,
        loop_signal: LoopSignal,
        socket_name: String,
    ) -> anyhow::Result<Self> {
        let compositor_state = CompositorState::new::<Self>(dh);
        let xdg_shell_state = XdgShellState::new::<Self>(dh);
        let shm_state = ShmState::new::<Self>(dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(dh);
        let data_device_state = DataDeviceState::new::<Self>(dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(dh, identity.seat_name);
        // KeyboardError embeds non-Send/Sync xkb handles, so map it to a string error.
        seat.add_keyboard(Default::default(), 200, 200)
            .map_err(|e| anyhow::anyhow!("failed to add keyboard: {e}"))?;
        seat.add_pointer();

        // Output identity comes from the backend; mapped at the space origin.
        let mode = identity.mode;
        let output = Output::new(identity.output_name, identity.physical);
        let output_global = output.create_global::<Self>(dh);
        output.change_current_state(Some(mode), Some(identity.transform), None, Some((0, 0).into()));
        output.set_preferred(mode);

        let mut space = Space::<Window>::default();
        space.map_output(&output, (0, 0));
        let damage_tracker = OutputDamageTracker::from_output(&output);

        let spawn_command = std::env::var("MUNDANE_TERMINAL").ok();
        let spawn_settings_command = std::env::var("MUNDANE_SETTINGS").ok();

        let mut animation = AnimationEngine::new();
        let wallpaper = WallpaperManager::new(&config.wallpaper, &mut animation);

        Ok(Self {
            display_handle: dh.clone(),
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            seat,
            space,
            output,
            damage_tracker,
            _output_global: output_global,
            animation,
            wm: WindowManager::new(),
            wallpaper,
            config,
            cursor_status: CursorImageStatus::default_named(),
            cursor_location: Point::from((0.0, 0.0)),
            move_state: None,
            start_time: Instant::now(),
            loop_signal,
            socket_name,
            spawn_command,
            spawn_settings_command,
        })
    }

    /// Reload the configured wallpaper, crossfading from the current one (Super+W).
    pub fn reload_wallpaper(&mut self) {
        if let Some(path) = self.config.wallpaper.path.clone() {
            self.wallpaper.set_wallpaper(&path, &mut self.animation);
        }
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        self.handle_commit(surface);
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl OutputHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
