//! The compositor state and its Wayland protocol handler implementations.
//!
//! [`State`] owns every piece of Wayland protocol state, the desktop [`Space`], the
//! animation engine and the magnetic window manager. The handler traits below wire the
//! Smithay-managed protocols (compositor, shm, xdg-shell, seat, data-device, output) to
//! this state; the `delegate_*!` macros generate the dispatch glue.

use smithay::desktop::PopupManager;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::wayland::selection::data_device::set_data_device_focus;
use smithay::wayland::selection::primary_selection::{set_primary_focus, PrimarySelectionState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::xdg_activation::XdgActivationState;
use std::collections::VecDeque;
use std::os::unix::io::OwnedFd;
use std::time::Instant;


use smithay::desktop::{Space, Window};
use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatHandler, SeatState};

use smithay::reexports::calloop::{LoopHandle, LoopSignal, RegistrationToken};
use smithay::reexports::wayland_server::backend::{
    ClientData, ClientId, DisconnectReason,
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle};
use smithay::utils::{Logical, Point};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::xwayland_shell::XWaylandShellState;
use smithay::xwayland::{X11Wm, XWaylandClientData};

use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
};

use crate::animation::AnimationEngine;
use crate::config::MundaneConfig;
use crate::wallpaper::WallpaperManager;
use crate::wm::WindowManager;

/// An in-progress interactive window move (Super + left drag).
pub struct MoveState {
    pub client_grab: bool,
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
pub struct ClientState {
    pub context: Option<smithay::wayland::security_context::SecurityContext>,
    pub on_exit: calloop::ping::Ping,
    pub compositor_state: CompositorClientState,
    pub role: crate::shell::services::Role,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) { self.on_exit.ping(); }
}

pub struct State {
    pub desktop: crate::shell::desktop::Desktop,
    pub display_handle: DisplayHandle,
    pub client_exit: calloop::ping::Ping,

    // Wayland protocol globals.
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub seat: Seat<Self>,
    pub handle: LoopHandle<'static, Self>,
    pub xwayland_shell: XWaylandShellState,
    pub xwm: Option<X11Wm>,
    pub native_offers: [Option<Vec<String>>; 2],
    pub x11_waiters: Vec<std::os::unix::net::UnixStream>,
    pub xserver: Option<(RegistrationToken, u32)>,
    pub popups: PopupManager,
    pub primary: PrimarySelectionState,
    pub clipboard_control: smithay::wayland::selection::ext_data_control::DataControlState,
    pub services: Vec<crate::shell::services::Helper>,
    pub activation: XdgActivationState,
    pub minimized: Vec<Window>,
    pub recent_input: VecDeque<(Serial, ClientId, Instant)>,
    // Retained to keep the wl_output / xdg-output manager globals alive.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,

    // Desktop layout + output.
    pub space: Space<Window>,
    pub session_channel: Option<std::os::unix::net::UnixStream>,
    pub switch_vt: Option<Box<dyn Fn(i32)>>,
    pub generation: u64,
    pub output_order: Vec<String>,
    pub layout: Vec<(smithay::output::Output,smithay::utils::Rectangle<i32,Logical>)>,
    pub dmabuf: smithay::wayland::dmabuf::DmabufState,
    pub imports: Vec<(smithay::backend::allocator::dmabuf::Dmabuf, smithay::wayland::dmabuf::ImportNotifier)>,
    pub clock: smithay::utils::Clock<smithay::utils::Monotonic>,

    // mundaneDE-specific.
    pub animation: AnimationEngine,
    pub wm: WindowManager,
    pub wallpaper: WallpaperManager,
    pub config: MundaneConfig,

    // Cache the themed buffer until the requested shape changes.
    pub named_cursor: crate::cursor::NamedCursor,
    pub cursor_status: CursorImageStatus,
    /// Authoritative pointer position. Updated for every motion event regardless of any
    /// in-progress interactive grab; the seat's `pointer.current_location()` can't serve as
    /// the source of truth because we deliberately skip `pointer.motion(...)` during a drag
    /// (otherwise the grabbed window would receive stray motion events), which freezes its
    /// reported location and breaks the libinput relative-motion accumulator (`current +
    /// delta`).
    pub cursor_location: Point<f64, Logical>,

    // Interactive move (manual grab — see [`MoveState`]).
    pub window_menu: Option<crate::shell::menu::WindowMenu>,
    pub dnd_icon: Option<WlSurface>,
    pub resize: Option<crate::shell::interaction::Resize>,
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
        seat_name: String,
        loop_signal: LoopSignal,
        handle: LoopHandle<'static, Self>,
        socket_name: String,
    ) -> anyhow::Result<Self> {
        let clock = smithay::utils::Clock::<smithay::utils::Monotonic>::new();
        crate::tearing::register(dh);
        smithay::wayland::presentation::PresentationState::new::<Self>(dh, clock.id() as u32);
        smithay::wayland::viewporter::ViewporterState::new::<Self>(dh);
        smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(dh);
        let compositor_state = CompositorState::new::<Self>(dh);
        let xdg_shell_state = XdgShellState::new::<Self>(dh);
        XdgDecorationState::new::<Self>(dh);
        smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(dh);
        TextInputManagerState::new::<Self>(dh);
        let shm_state = ShmState::new::<Self>(dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(dh);
        let data_device_state = DataDeviceState::new::<Self>(dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(dh, seat_name);
        // KeyboardError embeds non-Send/Sync xkb handles, so map it to a string error.
        seat.add_keyboard(Default::default(), 200, 200)
            .map_err(|e| anyhow::anyhow!("failed to add keyboard: {e}"))?;
        seat.add_pointer();

        let spawn_command = std::env::var("MUNDANE_TERMINAL").ok();
        let spawn_settings_command = std::env::var("MUNDANE_SETTINGS").ok();

        let mut animation = AnimationEngine::new();
        let wallpaper = WallpaperManager::new(&config.wallpaper, &mut animation);

        let primary = PrimarySelectionState::new::<Self>(dh);
        let clipboard_control = crate::shell::services::globals(dh, &primary);
        let (client_exit, receiver) = calloop::ping::make_ping()?;
        handle.insert_source(receiver, |_,_,state| {
            state.generation += 1;
            if state.native_offers.iter().all(Option::is_none) { return; }
            // The source can die after the focused window receives its last offer.
            // Re-offer both selections after Wayland has torn down the client's resources.
            let client = state.seat.get_keyboard().and_then(|k| k.current_focus())
                .and_then(|f| f.surface.client());
            set_primary_focus(&state.display_handle, &state.seat, None);
            set_data_device_focus(&state.display_handle, &state.seat, None);
            set_primary_focus(&state.display_handle, &state.seat, client.clone());
            set_data_device_focus(&state.display_handle, &state.seat, client);
        }).map_err(|e|anyhow::anyhow!("register client exit notification: {e}"))?;

        Ok(Self {
            desktop: crate::shell::desktop::Desktop::new(dh,handle.clone()),
            display_handle: dh.clone(),
            client_exit,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            seat,
            space: Space::default(),
            session_channel: None,
            switch_vt: None,
            generation: 1,
            output_order: Vec::new(),
            layout: Vec::new(),
            dmabuf: smithay::wayland::dmabuf::DmabufState::new(),
            imports: Vec::new(),
            clock,
            handle,
            xwayland_shell: XWaylandShellState::new::<Self>(dh),
            xwm: None,
            native_offers: Default::default(),
            x11_waiters: Vec::new(),
            xserver: None,
            popups: PopupManager::default(),
            minimized: Vec::new(),
            recent_input: VecDeque::new(),
            primary,
            clipboard_control,
            services: Vec::new(),
            activation: XdgActivationState::new::<Self>(dh),
            animation,
            wm: WindowManager::new(),
            wallpaper,
            config,
            named_cursor: crate::cursor::NamedCursor::load(Default::default()),
            cursor_status: CursorImageStatus::default_named(),
            cursor_location: Point::from((0.0, 0.0)),
            window_menu: None,
            resize: None,
            dnd_icon: None,
            move_state: None,
            start_time: Instant::now(),
            loop_signal,
            socket_name,
            spawn_command,
            spawn_settings_command,
        })
    }

    pub fn request_logout(&self) {
        use std::io::Write;
        if let Some(mut leader) = self.session_channel.as_ref() {
            if leader.write_all(b"Q").is_ok() {
                // Keep serving Wayland until the leader stops graphical units.
                return;
            }
        }
        self.loop_signal.stop();
    }

    pub fn launch_helpers(&mut self) -> anyhow::Result<()> {
        self.supervise_shell()?;
        self.services = crate::shell::services::launch(&self.display_handle, &self.client_exit)?;
        Ok(())
    }

    /// Reload the configured wallpaper, crossfading from the current one (Super+W).
    pub fn reload_wallpaper(&mut self) {
        tracing::trace!(target: "mundane_frame", "wallpaper reload");
        self.wallpaper
            .set_wallpaper(self.config.wallpaper.path.as_deref(), &mut self.animation);
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn new_surface(&mut self, surface:&WlSurface) {
        if surface.client().and_then(|c|c.get_data::<ClientState>().map(|d|d.role))==Some(crate::shell::services::Role::Shell){
            crate::shell::desktop::track_layer_teardown(surface);
        }
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(x) = client.get_data::<XWaylandClientData>() {
            return &x.compositor_state;
        }
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        self.generation += 1;
        self.handle_commit(surface);
        self.layer_commit(surface);
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
    type KeyboardFocus = crate::focus::KeyboardFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&Self::KeyboardFocus>) {
        let client =
            focused.and_then(|surface| self.display_handle.get_client(surface.surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client.clone());
        set_primary_focus(&self.display_handle, seat, client);
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        if let CursorImageStatus::Named(icon) = image {
            if icon != self.named_cursor.icon {
                self.named_cursor = crate::cursor::NamedCursor::load(icon);
            }
        }
        self.cursor_status = image;
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {
    fn started(
        &mut self,
        _: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<WlSurface>,
        _: Seat<Self>,
    ) {
        self.dnd_icon = icon;
    }
    fn dropped(&mut self, _: Option<WlSurface>, _: bool, _: Seat<Self>) {
        self.dnd_icon = None;
    }
}
impl ServerDndGrabHandler for State {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl OutputHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
