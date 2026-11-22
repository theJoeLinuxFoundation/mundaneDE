//! Start the rootless X server when an application launcher requests its display.
use crate::state::State;
use anyhow::{Context, Result};
use calloop::{generic::Generic, Interest, Mode, PostAction};
use smithay::{
    desktop::Window,
    input::Seat,
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            data_device::*, primary_selection::*, SelectionHandler, SelectionSource,
            SelectionTarget,
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
    },
};
use std::{
    cell::Cell,
    io::{Read, Write},
    os::unix::{
        io::OwnedFd,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::Stdio,
};

fn endpoint() -> Result<PathBuf> {
    let socket = std::env::var("WAYLAND_DISPLAY")?;
    Ok(
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("missing XDG_RUNTIME_DIR")?)
            .join(format!("{socket}.x11")),
    )
}
pub fn print_display() -> Result<()> {
    let mut reply = String::new();
    let mut stream =
        UnixStream::connect(endpoint()?).context("connect to mundaneDE X11 launcher")?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    stream.read_to_string(&mut reply)?;
    anyhow::ensure!(
        reply
            .strip_prefix(':')
            .and_then(|n| n.trim().parse::<u32>().ok())
            .is_some(),
        "Xwayland unavailable: {reply}"
    );
    print!("{reply}");
    Ok(())
}
pub struct ListenerPath(PathBuf);
impl Drop for ListenerPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn listen(state: &State) -> Result<ListenerPath> {
    let path = endpoint()?;
    // The Wayland listening socket and its lock were acquired before this point.
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    let guard = ListenerPath(path);
    state
        .handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            |_, listener, state| {
                let (mut conn, _) = listener.accept()?;
                match state.xdisplay() {
                    Ok(n) if state.xwm.is_some() => {
                        let _ = writeln!(conn, ":{n}");
                    }
                    Ok(_) => state.x11_waiters.push(conn),
                    Err(e) => {
                        let _ = writeln!(conn, "{e}");
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("register X11 launcher: {e}"))?;
    Ok(guard)
}

#[derive(Default)]
struct FocusedOnce(Cell<bool>);

impl State {
    pub fn x11_focused_once(&self, w: &Window) -> bool {
        w.user_data().insert_if_missing(FocusedOnce::default);
        w.user_data().get::<FocusedOnce>().unwrap().0.replace(true)
    }
    fn xdisplay(&mut self) -> Result<u32> {
        if let Some((_, n)) = self.xserver {
            return Ok(n);
        }
        let env = [
            "HOME",
            "TMPDIR",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XKB_CONFIG_ROOT",
            "LD_LIBRARY_PATH",
            "LIBGL_ALWAYS_SOFTWARE",
            "GALLIUM_DRIVER",
            "__EGL_VENDOR_LIBRARY_FILENAMES",
        ]
        .into_iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k, v)));
        let (server, client) = XWayland::spawn(
            &self.display_handle,
            None,
            env,
            true,
            Stdio::null(),
            Stdio::inherit(),
            |_| {},
        )?;
        let number = server.display_number();
        let token = self
            .handle
            .insert_source(server, move |event, _, state| {
                if let XWaylandEvent::Ready { x11_socket, .. } = event {
                    match X11Wm::start_wm(state.handle.clone(), x11_socket, client.clone()) {
                        Ok(mut wm) => {
                            for target in [SelectionTarget::Clipboard, SelectionTarget::Primary] {
                                if let Some(mimes) = &state.native_offers[selection_index(target)] {
                                    let _ = wm.new_selection(target, Some(mimes.clone()));
                                }
                            }
                            state.xwm = Some(wm);
                            for mut client in state.x11_waiters.drain(..) {
                                let _ = writeln!(client, ":{number}");
                            }
                            return;
                        }
                        Err(e) => tracing::warn!(%e,"could not start X11 window manager"),
                    }
                }
                state.release_xserver();
            })
            .map_err(|e| anyhow::anyhow!("register Xwayland: {e}"))?;
        self.xserver = Some((token, number));
        Ok(number)
    }
    fn release_xserver(&mut self) {
        if let Some((source, _)) = self.xserver.take() {
            self.handle.remove(source);
        }
        self.xwm = None;
        // Xwayland starts its surface serials at one again after a restart.
        // Discard the previous server's serial-to-surface associations with its global.
        self.display_handle
            .remove_global::<State>(self.xwayland_shell.global());
        self.xwayland_shell = XWaylandShellState::new::<State>(&self.display_handle);
        self.x11_waiters.clear();
        tracing::info!("Xwayland stopped");
    }
    fn xwindow(&self, x: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.x11_surface() == Some(x))
            .cloned()
    }
}
impl XWaylandShellHandler for State {
    fn surface_associated(
        &mut self,
        _: XwmId,
        _: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        x: X11Surface,
    ) {
        // X11's serial message and the Wayland buffer commit travel on separate sockets.
        self.handle.insert_idle(move |state| {
            if let Some(w) = state.xwindow(&x) {
                w.on_commit();
                if !x.is_override_redirect()
                    && x.wl_surface().is_some_and(|s| {
                        smithay::backend::renderer::utils::with_renderer_surface_state(&s, |s| {
                            s.buffer().is_some()
                        })
                        .unwrap_or(false)
                    })
                    && !state.x11_focused_once(&w)
                {
                    crate::focus::set_keyboard_focus(state, Some(&w));
                }
            }
        });
    }
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell
    }
}
smithay::delegate_xwayland_shell!(State);

impl XwmHandler for State {
    fn xwm_state(&mut self, _: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }
    fn new_window(&mut self, _: XwmId, _: X11Surface) {}
    fn new_override_redirect_window(&mut self, _: XwmId, _: X11Surface) {}
    fn map_window_request(&mut self, _: XwmId, x: X11Surface) {
        let mut geo = x.geometry();
        let parent = x
            .is_transient_for()
            .and_then(|id| {
                self.space
                    .elements()
                    .find(|w| w.x11_surface().is_some_and(|s| s.window_id() == id))
            })
            .and_then(|w| self.space.element_geometry(w));
        geo.loc = parent
            .map(|p| {
                (
                    p.loc.x + (p.size.w - geo.size.w) / 2,
                    p.loc.y + (p.size.h - geo.size.h) / 2,
                )
                    .into()
            })
            .unwrap_or_else(|| {
                self.wm
                    .initial_placement(geo.size, self.pointer_bounds(), &self.space)
            });
        if x.set_mapped(true).is_ok() && x.configure(geo).is_ok() {
            self.space
                .map_element(Window::new_x11_window(x), geo.loc, true);
        }
    }
    fn mapped_override_redirect_window(&mut self, _: XwmId, x: X11Surface) {
        let pos = x.geometry().loc;
        self.space
            .map_element(Window::new_x11_window(x), pos, false);
    }
    fn unmapped_window(&mut self, _: XwmId, x: X11Surface) {
        if let Some(w) = self.xwindow(&x) {
            self.space.unmap_elem(&w);
            if !x.is_override_redirect() {
                let _ = x.set_mapped(false);
                crate::focus::refocus(self);
            }
        }
    }
    fn destroyed_window(&mut self, id: XwmId, x: X11Surface) {
        self.unmapped_window(id, x);
    }
    fn configure_request(
        &mut self,
        _: XwmId,
        x: X11Surface,
        _: Option<i32>,
        _: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _: Option<Reorder>,
    ) {
        let mut geo = x.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = x.configure(geo);
    }
    fn configure_notify(
        &mut self,
        _: XwmId,
        x: X11Surface,
        geo: Rectangle<i32, Logical>,
        _: Option<u32>,
    ) {
        if let Some(w) = self.xwindow(&x) {
            self.space.map_element(w, geo.loc, false);
        }
    }
    fn resize_request(&mut self, _: XwmId, _: X11Surface, _: u32, _: ResizeEdge) {}
    fn move_request(&mut self, _: XwmId, _: X11Surface, _: u32) {}
    fn allow_selection_access(&mut self, id: XwmId, _: SelectionTarget) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .and_then(|f| f.x11)
            .and_then(|x| x.xwm_id())
            == Some(id)
    }
    fn send_selection(&mut self, _: XwmId, ty: SelectionTarget, mime: String, fd: OwnedFd) {
        let result = match ty {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime, fd)
                    .map_err(|e| e.to_string())
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime, fd).map_err(|e| e.to_string())
            }
        };
        if let Err(e) = result {
            tracing::debug!(%e,"Wayland selection transfer ended");
        }
    }
    fn new_selection(&mut self, id: XwmId, ty: SelectionTarget, mimes: Vec<String>) {
        if !self.allow_selection_access(id, ty) {
            return;
        }
        self.native_offers[selection_index(ty)] = None;
        match ty {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mimes, id)
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mimes, id)
            }
        }
    }
    fn cleared_selection(&mut self, id: XwmId, ty: SelectionTarget) {
        match ty {
            SelectionTarget::Clipboard
                if current_data_device_selection_userdata(&self.seat).as_deref() == Some(&id) =>
            {
                clear_data_device_selection(&self.display_handle, &self.seat)
            }
            SelectionTarget::Primary
                if current_primary_selection_userdata(&self.seat).as_deref() == Some(&id) =>
            {
                clear_primary_selection(&self.display_handle, &self.seat)
            }
            _ => {}
        }
    }
    fn disconnected(&mut self, id: XwmId) {
        self.cleared_selection(id, SelectionTarget::Clipboard);
        self.cleared_selection(id, SelectionTarget::Primary);
        self.release_xserver();
    }
}
impl SelectionHandler for State {
    type SelectionUserData = XwmId;
    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _: Seat<Self>,
    ) {
        let offer = source.map(|s| s.mime_types());
        if let Some(x) = &mut self.xwm {
            let _ = x.new_selection(ty, offer.clone());
        }
        self.native_offers[selection_index(ty)] = offer;
    }
    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime: String,
        fd: OwnedFd,
        _: Seat<Self>,
        id: &XwmId,
    ) {
        if let Some(x) = self.xwm.as_mut().filter(|x| x.id() == *id) {
            if let Err(e) = x.send_selection(ty, mime, fd, self.handle.clone()) {
                tracing::warn!(%e,"X11 selection transfer failed");
            }
        }
    }
}

fn selection_index(target: SelectionTarget) -> usize {
    match target {
        SelectionTarget::Clipboard => 0,
        SelectionTarget::Primary => 1,
    }
}
