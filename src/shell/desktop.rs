//! Shell surfaces, workspace ownership and the compositor-issued control connection.
use super::lock::Lock;
use crate::shell::services::Role;
use crate::state::{ClientState, State};
use smithay::{
    desktop::{layer_map_for_output, LayerSurface, Window, WindowSurfaceType},
    output::Output,
    reexports::wayland_server::{
        protocol::{wl_output::WlOutput, wl_surface::WlSurface},
        DisplayHandle,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        foreign_toplevel_list::{
            ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
        },
        idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState},
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        security_context::{
            SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
            SecurityContextState,
        },
        shell::{
            wlr_layer::{
                KeyboardInteractivity, Layer, LayerSurface as ProtocolLayer, LayerSurfaceData,
                WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::XdgToplevelSurfaceData,
        },
    },
};
use std::{collections::HashMap, os::unix::net::UnixDatagram, sync::Arc};

pub struct Task {
    pub handle: ForeignToplevelHandle,
    pub workspace: u8,
    position: Point<i32, Logical>,
    stack_index: usize,
}
pub struct WorkspaceTransition {
    pub spring: crate::animation::SpringId,
    pub previous: Vec<(Window, Rectangle<i32, Logical>)>,
}

pub struct Desktop {
    layers: WlrLayerShellState,
    list: ForeignToplevelListState,
    idle: IdleNotifierState<State>,
    inhibitors: Vec<WlSurface>,
    pub presented: HashMap<Output, smithay::backend::renderer::element::RenderElementStates>,
    pub lock: Lock,
    pub switcher: Option<super::switcher::Switcher>,
    pub switcher_keys: std::collections::HashSet<u32>,
    pub menu_surface: Option<WlSurface>,
    previous_menu_focus: Option<crate::focus::KeyboardFocus>,
    pub dismissed_buttons: std::collections::HashSet<u32>,
    pub monitor: super::keyboard_monitor::KeyboardMonitor,
    pub pending_keys: std::collections::VecDeque<super::keyboard_monitor::PendingKey>,
    pub reduce_motion: bool,
    pub tasks: HashMap<Window, Task>,
    pub workspace: u8,
    pub transition: Option<WorkspaceTransition>,
    pub showing_desktop: bool,
    pub channel: Option<UnixDatagram>,
    pub watch_tasks: bool,
    pub active_task: Option<Window>,
    pub panel: Option<std::process::Child>,
    pub panel_source: Option<calloop::RegistrationToken>,
}
impl Desktop {
    pub fn new(dh: &DisplayHandle, handle: calloop::LoopHandle<'static, State>) -> Self {
        SecurityContextState::new::<State, _>(dh, |c| {
            c.get_data::<ClientState>()
                .is_some_and(|d| d.context.is_none())
        });
        IdleInhibitManagerState::new::<State>(dh);
        Self {
            pending_keys: Default::default(),
            monitor: Default::default(),
            reduce_motion: false,
            layers: WlrLayerShellState::new_with_filter::<State, _>(dh, |c| {
                super::services::role(c) == Role::Shell
            }),
            list: ForeignToplevelListState::new::<State>(dh),
            idle: IdleNotifierState::new(dh, handle),
            inhibitors: vec![],
            presented: HashMap::new(),
            lock: Lock::new(dh),
            menu_surface: None,
            switcher: None,
            switcher_keys: Default::default(),
            previous_menu_focus: None,
            dismissed_buttons: Default::default(),
            tasks: HashMap::new(),
            transition: None,
            workspace: 1,
            showing_desktop: false,
            channel: None,
            panel: None,
            watch_tasks: false,
            active_task: None,
            panel_source: None,
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(mut child) = self.panel.take() {
            if let Some(channel) = &self.channel {
                let _ = channel.send(br#"{"logout":true}"#);
                let until = std::time::Instant::now() + std::time::Duration::from_millis(750);
                while std::time::Instant::now() < until {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl State {
    pub fn register_task(&mut self, w: &Window) {
        if self.desktop.tasks.contains_key(w)
            || w.x11_surface().is_some_and(|x| x.is_override_redirect())
        {
            return;
        }
        let (title, app) = Self::task_labels(w);
        self.desktop.tasks.insert(
            w.clone(),
            Task {
                handle: self.desktop.list.new_toplevel::<Self>(title, app),
                workspace: self.desktop.workspace,
                position: self.space.element_location(w).unwrap_or_default(),
                stack_index: self.space.elements().count(),
            },
        );
        self.publish_tasks();
    }
    pub fn publish_tasks(&mut self) {
        if self.desktop.watch_tasks {
            self.shell_command("list");
        }
    }
    fn task_labels(w: &Window) -> (String, String) {
        if let Some(x) = w.x11_surface() {
            return (x.title(), x.class());
        }
        w.toplevel()
            .map(|t| {
                with_states(t.wl_surface(), |s| {
                    let a = s
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap();
                    (
                        a.title.clone().unwrap_or_default(),
                        a.app_id.clone().unwrap_or_default(),
                    )
                })
            })
            .unwrap_or_default()
    }
    pub fn task_changed(&mut self, w: &Window) {
        if let Some(task) = self.desktop.tasks.get(w) {
            let (title, app) = Self::task_labels(w);
            if title != task.handle.title() || app != task.handle.app_id() {
                task.handle.send_title(&title);
                task.handle.send_app_id(&app);
                task.handle.send_done();
                self.publish_tasks();
            }
        }
    }
    pub fn forget_task(&mut self, w: &Window) {
        self.switcher_lost_window(w);
        if let Some(task) = self.desktop.tasks.remove(w) {
            self.revoke_capture(Some(&crate::capture::Source::Window(
                task.handle.identifier(),
            )));
            self.desktop.list.remove_toplevel(&task.handle);
            self.publish_tasks();
        }
    }
    fn arrange_workspace(&mut self) {
        let visible: Vec<_> = self.space.elements().cloned().collect();
        for (index, w) in visible.iter().enumerate() {
            if let Some(task) = self.desktop.tasks.get_mut(w) {
                task.stack_index = index;
                task.position = self.space.element_location(w).unwrap_or(task.position);
                if self.desktop.showing_desktop || task.workspace != self.desktop.workspace {
                    self.space.unmap_elem(w);
                }
            }
        }
        if !self.desktop.showing_desktop {
            let mut restore: Vec<_> = self
                .desktop
                .tasks
                .iter()
                .filter(|(w, t)| {
                    t.workspace == self.desktop.workspace
                        && !self.minimized.contains(w)
                        && self.space.element_location(w).is_none()
                })
                .collect();
            restore.sort_by_key(|(_, t)| t.stack_index);
            for (window, task) in restore {
                self.space.map_element(window.clone(), task.position, false);
            }
        }
        self.generation += 1;
        crate::focus::refocus(self);
    }
    pub fn leave_desktop(&mut self) {
        if self.desktop.showing_desktop {
            self.desktop.showing_desktop = false;
            self.arrange_workspace();
        }
    }
    pub fn toggle_desktop(&mut self) {
        if self.desktop.showing_desktop {
            self.leave_desktop();
            return;
        }
        let output = self
            .space
            .output_under(self.cursor_location)
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned();
        let Some(output) = output else { return };
        let icons = layer_map_for_output(&output)
            .layers()
            .find(|l| l.namespace() == "mundane-desktop-icons")
            .map(|l| l.wl_surface().clone());
        if let Some(surface) = icons {
            self.desktop.showing_desktop = true;
            self.arrange_workspace();
            self.layer_focus(&surface);
            self.shell_message(&serde_json::json!({"desktop_icons":output.name()}));
        }
    }
    pub fn switch_workspace(&mut self, n: u8) {
        if (1..=4).contains(&n) && self.desktop.workspace != n {
            self.finish_switcher(false);
            self.desktop.showing_desktop = false;
            if let Some(old) = self.desktop.transition.take() {
                self.animation.remove_spring(old.spring);
            }
            if !self.config.reduced_motion && !self.desktop.reduce_motion {
                let previous = self
                    .space
                    .elements()
                    .rev()
                    .filter_map(|w| Some((w.clone(), self.space.element_geometry(w)?)))
                    .collect();
                let spring = self.animation.create_spring_kind(
                    0,
                    0,
                    crate::animation::SpringParams {
                        stiffness: 1250.,
                        damping: 69.,
                        mass: 1.,
                    },
                    crate::animation::SpringKind::Fade,
                );
                self.animation
                    .spring_mut(spring)
                    .unwrap()
                    .set_target(100, 0);
                self.desktop.transition = Some(WorkspaceTransition { spring, previous });
            }
            self.desktop.workspace = n;
            self.arrange_workspace();
            self.shell_command("list");
        }
    }
    pub fn shell_message(&self, msg: &serde_json::Value) {
        if let Some(sock) = &self.desktop.channel {
            let _ = sock.send(msg.to_string().as_bytes());
        }
    }
    pub fn shell_command(&mut self, command: &str) {
        if let Some(serial) = command
            .strip_prefix("keyboard-forward ")
            .and_then(|s| s.parse::<u32>().ok())
        {
            self.forward_reader_keys(serial.into());
            return;
        }
        if let Some(data) = command.strip_prefix("keyboard-monitor ") {
            if let Ok(next) = serde_json::from_str(data) {
                self.desktop.monitor.configure(next);
            }
            return;
        }
        if let Some(value) = command.strip_prefix("reduced-motion ") {
            self.desktop.reduce_motion = value == "on";
            self.generation += 1;
            return;
        }
        let mut args = command.split_ascii_whitespace();
        let op = args.next().unwrap_or("");
        // A replacement panel may subscribe during lock. Publish its snapshot on unlock.
        if op == "watch-tasks" {
            self.desktop.watch_tasks = true;
            self.shell_message(&serde_json::json!({"locked": self.desktop.lock.active}));
        }
        if self.desktop.lock.active {
            return;
        }
        match op {
            "watch-tasks" | "list" => {
                let tasks: Vec<_> = self
                    .desktop
                    .tasks
                    .iter()
                    .map(|(w, t)| {
                        let mut task = serde_json::json!({
                            "id":t.handle.identifier(),
                            "workspace":t.workspace,"minimized":self.minimized.contains(w)
                        });
                        if self.desktop.watch_tasks {
                            task["active"] = (self.desktop.active_task.as_ref() == Some(w)).into();
                            task["title"] = t
                                .handle
                                .title()
                                .chars()
                                .take(512)
                                .collect::<String>()
                                .into();
                            task["app"] = t.handle.app_id().into();
                        }
                        task
                    })
                    .collect();
                let outputs: Vec<_> = self.space.outputs().map(|output| output.name()).collect();
                self.shell_message(
                    &serde_json::json!({"workspace":self.desktop.workspace,"tasks":tasks,"outputs":outputs}),
                );
            }
            "focus-taskbar" | "leave-taskbar"
                if self.desktop.watch_tasks && self.desktop.menu_surface.is_none() =>
            {
                let bar = self
                    .space
                    .output_under(self.cursor_location)
                    .next()
                    .or_else(|| self.space.outputs().next())
                    .and_then(|o| {
                        layer_map_for_output(o)
                            .layers()
                            .find(|l| l.namespace() == "mundane-taskbar")
                            .map(|l| l.wl_surface().clone())
                    });
                if let Some(bar) = bar {
                    if op == "focus-taskbar" {
                        self.layer_focus(&bar);
                    } else if self
                        .seat
                        .get_keyboard()
                        .and_then(|k| k.current_focus())
                        .is_some_and(|f| f.surface == bar)
                    {
                        crate::focus::refocus(self);
                    }
                }
            }
            "lock" => self.begin_lock(),
            "logout" => self.request_logout(),
            "next-workspace" => self.switch_workspace(self.desktop.workspace % 4 + 1),
            "previous-workspace" => self.switch_workspace((self.desktop.workspace + 2) % 4 + 1),
            "workspace" => {
                if let Some(n) = args.next().and_then(|s| s.parse().ok()) {
                    self.switch_workspace(n);
                }
            }
            "activate" | "move" | "close" => {
                let Some(id) = args.next() else { return };
                let Some(w) = self
                    .desktop
                    .tasks
                    .iter()
                    .find(|(_, t)| t.handle.identifier() == id)
                    .map(|(w, _)| w.clone())
                else {
                    return;
                };
                match op {
                    "move" => {
                        if let Some(n) = args
                            .next()
                            .and_then(|s| s.parse::<u8>().ok())
                            .filter(|n| (1..=4).contains(n))
                        {
                            self.desktop.tasks.get_mut(&w).unwrap().workspace = n;
                            self.arrange_workspace();
                            self.publish_tasks();
                        }
                    }
                    "close" => {
                        if let Some(t) = w.toplevel() {
                            t.send_close()
                        } else if let Some(x) = w.x11_surface() {
                            let _ = x.close();
                        }
                    }
                    _ => {
                        self.leave_desktop();
                        self.switch_workspace(self.desktop.tasks[&w].workspace);
                        if self.minimized.contains(&w) {
                            self.restore_window(&w);
                        } else {
                            crate::focus::set_keyboard_focus(self, Some(&w));
                        }
                    }
                }
                self.generation += 1;
            }
            _ => {}
        }
    }
    pub fn idle_inhibited(&mut self) -> bool {
        self.desktop.idle.is_inhibited()
    }
    pub fn input_activity(&mut self) {
        self.desktop.lock.last_input = std::time::Instant::now();
        self.desktop.idle.notify_activity(&self.seat);
    }
    pub fn update_inhibitors(&mut self) {
        use smithay::reexports::wayland_server::Resource;
        use smithay::wayland::seat::WaylandFocus;
        self.desktop.inhibitors.retain(|s| s.is_alive());
        let active = !self.desktop.lock.active
            && self.desktop.inhibitors.iter().any(|s| {
                let mut root = s.clone();
                while let Some(parent) = smithay::wayland::compositor::get_parent(&root) {
                    root = parent;
                }
                self.space.elements().any(|w| {
                    w.wl_surface().as_deref() == Some(&root)
                        && self.space.outputs_for_element(w).iter().any(|o| {
                            self.desktop
                                .presented
                                .get(o)
                                .is_some_and(|frame| frame.element_was_presented(s))
                        })
                })
            });
        self.desktop.idle.set_is_inhibited(active);
    }
    pub fn work_area(&self, w: &Window) -> Rectangle<i32, Logical> {
        let Some(o) = self.window_output(w) else {
            return self.window_bounds(w);
        };
        let mut r = layer_map_for_output(&o).non_exclusive_zone();
        if r.is_empty() {
            return self.window_bounds(w);
        }
        r.loc += self.window_bounds(w).loc;
        if super::decorations::decorated(w) {
            let height = super::decorations::HEIGHT.min(r.size.h - 1);
            r.loc.y += height;
            r.size.h -= height;
        }
        r
    }
    pub fn layer_commit(&mut self, s: &WlSurface) {
        let found = self.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            map.layer_for_surface(s, WindowSurfaceType::TOPLEVEL)
                .cloned()
                .map(|l| (o.clone(), l))
        });
        let Some((o, l)) = found else { return };
        let mut map = layer_map_for_output(&o);
        let old = map.non_exclusive_zone();
        map.arrange();
        let area = map.non_exclusive_zone();
        let resized = area != old;
        tracing::debug!(target: "mundane_shell_frames", output = o.name(), role = l.namespace(),
            width = area.size.w, height = area.size.h, "layer commit");
        drop(map);
        if !with_states(s, |s| {
            s.data_map
                .get::<LayerSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        }) {
            l.layer_surface().send_configure();
        }
        if !self.desktop.lock.active
            && l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive
        {
            self.finish_switcher(false);
            if matches!(
                l.namespace(),
                "mundane-start"
                    | "mundane-system"
                    | "mundane-overview"
                    | "mundane-app-windows"
                    | "mundane-tray"
                    | "mundane-power"
                    | "mundane-capture"
                    | "mundane-history"
            ) {
                if self.desktop.menu_surface.is_none() {
                    self.desktop.previous_menu_focus =
                        self.seat.get_keyboard().and_then(|k| k.current_focus());
                }
                self.desktop.menu_surface = Some(s.clone());
            }
            self.layer_focus(l.wl_surface());
        }
        if resized {
            self.resize_expanded_windows();
        }
    }
    pub fn layer_focus(&mut self, s: &WlSurface) {
        let k = self.seat.get_keyboard().unwrap();
        k.set_focus(
            self,
            Some(crate::focus::KeyboardFocus::native(s.clone())),
            SERIAL_COUNTER.next_serial(),
        );
    }
    pub fn hit_layer(
        &self,
        pos: Point<f64, Logical>,
        front: bool,
    ) -> Option<(WlSurface, Point<f64, Logical>, LayerSurface)> {
        let o = self.space.output_under(pos).next()?;
        let origin = self.space.output_geometry(o)?.loc;
        let map = layer_map_for_output(o);
        for layer in if front {
            [Layer::Overlay, Layer::Top]
        } else {
            [Layer::Bottom, Layer::Background]
        } {
            for l in map.layers_on(layer).rev() {
                if l.namespace() == "mundane-osd" {
                    continue;
                }
                let loc = origin + map.layer_geometry(l)?.loc;
                if let Some((s, off)) = l.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                {
                    return Some((s, (loc + off).to_f64(), l.clone()));
                }
            }
        }
        None
    }
}
// Smithay 0.7 resets the cached size when the role object dies, but its
// size-validation hook still runs on a later wl_surface commit. GTK detaches
// its buffer in exactly that order. Supply a valid inert size only for the
// retired role, which has already left the layer map.
#[derive(Default)]
struct RetiredLayer(std::sync::atomic::AtomicBool);
pub fn track_layer_teardown(surface: &WlSurface) {
    use smithay::wayland::compositor::add_pre_commit_hook;
    use smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState;
    add_pre_commit_hook::<State, _>(surface, |_, _, surface| {
        with_states(surface, |s| {
            if s.data_map
                .get::<RetiredLayer>()
                .is_some_and(|r| r.0.load(std::sync::atomic::Ordering::Relaxed))
            {
                s.cached_state
                    .get::<LayerSurfaceCachedState>()
                    .pending()
                    .size = (1, 1).into();
            }
        });
    });
}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.desktop.layers
    }
    fn new_layer_surface(&mut self, s: ProtocolLayer, o: Option<WlOutput>, _: Layer, name: String) {
        with_states(s.wl_surface(), |a| {
            a.data_map
                .insert_if_missing_threadsafe(RetiredLayer::default);
            a.data_map
                .get::<RetiredLayer>()
                .unwrap()
                .0
                .store(false, std::sync::atomic::Ordering::Relaxed);
        });
        if let Some(o) = o
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned())
        {
            let _ = layer_map_for_output(&o).map_layer(&LayerSurface::new(s, name));
        } else {
            s.send_close();
        }
    }
    fn layer_destroyed(&mut self, s: ProtocolLayer) {
        with_states(s.wl_surface(), |a| {
            if let Some(r) = a.data_map.get::<RetiredLayer>() {
                r.0.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
        let focused = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|f| &f.surface == s.wl_surface());
        for o in self.space.outputs() {
            let mut map = layer_map_for_output(o);
            let layer = map.layers().find(|l| l.layer_surface() == &s).cloned();
            if let Some(l) = layer {
                map.unmap_layer(&l);
            }
        }
        self.generation += 1;
        self.resize_expanded_windows();
        if self.desktop.menu_surface.as_ref() == Some(s.wl_surface()) {
            use smithay::utils::IsAlive;
            self.desktop.menu_surface = None;
            let old = self.desktop.previous_menu_focus.take();
            if focused && !self.desktop.lock.active {
                if old.as_ref().is_none_or(IsAlive::alive) {
                    self.seat.get_keyboard().unwrap().set_focus(
                        self,
                        old,
                        SERIAL_COUNTER.next_serial(),
                    );
                } else {
                    crate::focus::refocus(self);
                }
            }
        } else if focused && !self.desktop.lock.active {
            crate::focus::refocus(self);
        }
    }
}
impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.desktop.list
    }
}
impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.desktop.idle
    }
}
impl IdleInhibitHandler for State {
    fn inhibit(&mut self, s: WlSurface) {
        self.desktop.inhibitors.push(s);
        self.update_inhibitors();
    }
    fn uninhibit(&mut self, s: WlSurface) {
        if let Some(i) = self.desktop.inhibitors.iter().position(|x| x == &s) {
            self.desktop.inhibitors.swap_remove(i);
        }
        self.update_inhibitors();
    }
}
impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, c: SecurityContext) {
        let _ = self.handle.insert_source(source, move |stream, _, state| {
            let data = ClientState {
                role: Role::Application,
                context: Some(c.clone()),
                on_exit: state.client_exit.clone(),
                compositor_state: Default::default(),
            };
            let _ = state.display_handle.insert_client(stream, Arc::new(data));
        });
    }
}
smithay::delegate_layer_shell!(State);
smithay::delegate_foreign_toplevel_list!(State);
smithay::delegate_idle_notify!(State);
smithay::delegate_idle_inhibit!(State);
smithay::delegate_security_context!(State);
