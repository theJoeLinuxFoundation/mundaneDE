//! Output-local coordinates, client preferences and repaint notifications.
use crate::state::State;
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        egl::EGLDevice,
        renderer::{element::RenderElementStates, gles::GlesRenderer, ImportDma},
    },
    desktop::{
        utils::{surface_presentation_feedback_flags_from_states, OutputPresentationFeedback},
        Window,
    },
    output::{Mode, Output, PhysicalProperties, Scale},
    reexports::wayland_server::{backend::GlobalId, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Transform},
    wayland::{
        compositor::with_states,
        dmabuf::*,
        fractional_scale::{with_fractional_scale, FractionalScaleHandler},
    },
};

#[derive(Default)]
pub struct Position(pub std::cell::Cell<Option<[i32; 2]>>);

pub fn set_position(output: &Output, position: Option<[i32; 2]>) {
    output.user_data().insert_if_missing(Position::default);
    output
        .user_data()
        .get::<Position>()
        .unwrap()
        .0
        .set(position);
}
pub fn position(output: &Output) -> Option<[i32; 2]> {
    output.user_data().get::<Position>().and_then(|p| p.0.get())
}
pub fn valid_position(p: [i32; 2]) -> bool {
    p.into_iter().all(|v| (-32768..=32768).contains(&v))
}
#[derive(Default)]
struct PreferredOutput(std::cell::RefCell<Option<smithay::output::WeakOutput>>);
fn preferred(_: &WlSurface, data: &smithay::wayland::compositor::SurfaceData) -> Option<Output> {
    data.data_map
        .get::<PreferredOutput>()?
        .0
        .borrow()
        .as_ref()?
        .upgrade()
}
pub struct Head {
    pub output: Output,
    pub global: GlobalId,
}

impl State {
    pub fn add_head(
        &mut self,
        name: String,
        physical: PhysicalProperties,
        mode: Mode,
        transform: Transform,
    ) -> Head {
        let output = Output::new(name.clone(), physical);
        if !self.output_order.contains(&name) {
            self.output_order.push(name.clone());
        }
        set_position(
            &output,
            self.config
                .displays
                .get(&name)
                .and_then(|d| d.position)
                .filter(|p| valid_position(*p)),
        );
        let scale = self
            .config
            .displays
            .get(&name)
            .and_then(|v| v.scale)
            .filter(|v| valid_scale(*v))
            .unwrap_or(1.);
        output.change_current_state(
            Some(mode),
            Some(transform),
            Some(Scale::Fractional(scale)),
            None,
        );
        output.set_preferred(mode);
        self.space.map_output(&output, (0, 0));
        self.arrange_heads();
        let global = output.create_global::<Self>(&self.display_handle);
        Head { output, global }
    }

    pub fn remove_head(&mut self, head: Head) {
        self.revoke_capture(Some(&crate::capture::Source::Output(head.output.name())));
        self.desktop.lock.covered.remove(&head.output);
        self.desktop.presented.remove(&head.output);
        self.space.unmap_output(&head.output);
        // Let clients that saw the announcement bind an already in-flight wl_output.
        // New registry listeners see the removal immediately.
        self.display_handle
            .disable_global::<Self>(head.global.clone());
        let global = head.global;
        if self
            .handle
            .insert_source(
                calloop::timer::Timer::from_duration(std::time::Duration::from_secs(5)),
                {
                    let global = global.clone();
                    move |_, _, state| {
                        state.display_handle.remove_global::<Self>(global.clone());
                        calloop::timer::TimeoutAction::Drop
                    }
                },
            )
            .is_err()
        {
            self.display_handle.remove_global::<Self>(global);
        }
        self.arrange_heads();
    }

    pub fn arrange_heads(&mut self) {
        let previous = std::mem::take(&mut self.layout);
        let mut outputs: Vec<_> = self.space.outputs().cloned().collect();
        outputs.sort_by_key(|o| self.output_order.iter().position(|name| *name == o.name()));
        let mut x = 0;
        for output in outputs {
            let [left, top] = position(&output).unwrap_or([x, 0]);
            let loc = (left, top).into();
            self.space.map_output(&output, loc);
            output.change_current_state(None, None, None, Some(loc));
            let bounds = self.space.output_geometry(&output).unwrap();
            smithay::desktop::layer_map_for_output(&output).arrange();
            self.layout.push((output, bounds));
            x = x.max(bounds.loc.x + bounds.size.w);
        }
        self.sync_lock_outputs();
        self.generation += 1;
        self.window_menu = None;
        self.resize = None;
        self.move_state = None;
        if self.layout.is_empty() {
            return;
        }
        let old_pointer = self.cursor_location;
        if let Some((old, bounds)) = previous
            .iter()
            .find(|(_, r)| r.to_f64().contains(old_pointer))
        {
            if let Some(new) = self.space.output_geometry(old) {
                self.cursor_location += (new.loc - bounds.loc).to_f64();
            }
        }
        self.cursor_location = self.clamp_pointer(self.cursor_location);
        let windows: Vec<_> = self
            .space
            .elements()
            .chain(&self.minimized)
            .cloned()
            .collect();
        for window in &windows {
            let spring = self
                .wm
                .spring_id(window)
                .and_then(|id| self.animation.spring(id));
            let Some(pos) = self
                .space
                .element_location(window)
                .or_else(|| spring.map(|s| s.rounded_position().into()))
            else {
                continue;
            };
            let size = window.geometry().size;
            let rect = Rectangle::new(pos, size);
            let old = previous
                .iter()
                .filter_map(|(o, r)| {
                    let intersection = r.intersection(rect)?;
                    Some((
                        o,
                        r,
                        i64::from(intersection.size.w) * i64::from(intersection.size.h),
                    ))
                })
                .max_by_key(|(_, _, area)| *area);
            let bounds = old
                .and_then(|(o, _, _)| self.space.output_geometry(o))
                .unwrap_or_else(|| self.window_bounds(window));
            let delta = old.map(|(_, r, _)| bounds.loc - r.loc).unwrap_or_default();
            let mut target = clamp_window(pos + delta, size, bounds);
            if self.rehome_expanded(window, delta, bounds) {
                target = bounds.loc;
            }
            if target != pos {
                if self.minimized.contains(window) {
                    if let Some(s) = self
                        .wm
                        .spring_id(window)
                        .and_then(|id| self.animation.spring_mut(id))
                    {
                        s.set_target(target.x, target.y);
                        s.x.snap_to_target();
                        s.y.snap_to_target();
                    }
                } else {
                    self.place_window(window, target);
                }
            }
        }
        for w in windows.iter().filter(|w| !self.minimized.contains(w)) {
            self.space.raise_element(w, false);
        }
        self.space.refresh();
        self.pointer_moved(
            self.cursor_location,
            smithay::utils::SERIAL_COUNTER.next_serial(),
            self.start_time.elapsed().as_millis() as u32,
        );
        self.resize_expanded_windows();
        self.update_scales();
    }

    pub fn desktop_bounds(&self) -> Rectangle<i32, Logical> {
        self.space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .reduce(|a, b| a.merge(b))
            .unwrap_or_default()
    }
    pub fn clamp_pointer(&self, p: Point<f64, Logical>) -> Point<f64, Logical> {
        self.space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .map(|r| {
                let (x, y) = (r.loc.x as f64, r.loc.y as f64);
                let q = Point::from((
                    p.x.clamp(x, x + r.size.w as f64 - 0.001),
                    p.y.clamp(y, y + r.size.h as f64 - 0.001),
                ));
                (q, (q.x - p.x).hypot(q.y - p.y))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(q, _)| q)
            .unwrap_or_default()
    }
    pub fn pointer_bounds(&self) -> Rectangle<i32, Logical> {
        let o = self
            .space
            .output_under(self.cursor_location)
            .next()
            .or_else(|| self.space.outputs().next());
        o.and_then(|o| self.space.output_geometry(o))
            .unwrap_or_default()
    }
    fn output_for_rect(&self, rect: Rectangle<i32, Logical>) -> Option<&Output> {
        self.layout
            .iter()
            .filter_map(|(output, bounds)| {
                let overlap = bounds.intersection(rect)?;
                Some((
                    output,
                    i64::from(overlap.size.w) * i64::from(overlap.size.h),
                ))
            })
            .max_by_key(|(_, area)| *area)
            .map(|(output, _)| output)
    }
    pub fn window_output(&self, w: &Window) -> Option<Output> {
        self.space
            .element_geometry(w)
            .and_then(|r| self.output_for_rect(r))
            .or_else(|| self.space.output_under(self.cursor_location).next())
            .or_else(|| self.space.outputs().next())
            .cloned()
    }
    pub fn window_bounds(&self, w: &Window) -> Rectangle<i32, Logical> {
        self.window_output(w)
            .and_then(|o| self.space.output_geometry(&o))
            .unwrap_or_default()
    }
    pub fn update_scales(&self) {
        use smithay::{desktop::PopupManager, wayland::seat::WaylandFocus};
        for window in self.space.elements() {
            let Some(surface) = window.wl_surface() else {
                continue;
            };
            let Some(geometry) = self.space.element_geometry(window) else {
                continue;
            };
            self.scale_tree(&surface, geometry.loc - window.geometry().loc);
            for (popup, offset) in PopupManager::popups_for_surface(&surface) {
                self.scale_tree(
                    popup.wl_surface(),
                    geometry.loc + offset - popup.geometry().loc,
                );
            }
        }
    }
    fn scale_tree(&self, root: &WlSurface, origin: Point<i32, Logical>) {
        use smithay::{
            backend::renderer::utils::RendererSurfaceStateUserData,
            wayland::compositor::{with_surface_tree_downward, TraversalAction},
        };
        with_surface_tree_downward(
            root,
            origin,
            |surface, data, parent| {
                let view = data
                    .data_map
                    .get::<RendererSurfaceStateUserData>()
                    .and_then(|v| v.lock().unwrap().view());
                let Some(view) = view else {
                    return TraversalAction::SkipChildren;
                };
                let origin = *parent + view.offset;
                if let Some(output) = self.output_for_rect(Rectangle::new(origin, view.dst)) {
                    data.data_map.insert_if_missing(PreferredOutput::default);
                    let pref = data.data_map.get::<PreferredOutput>().unwrap();
                    pref.0.replace(Some(output.downgrade()));
                    let scale = output.current_scale();
                    with_fractional_scale(data, |s| {
                        s.set_preferred_scale(scale.fractional_scale())
                    });
                    smithay::wayland::compositor::send_surface_state(
                        surface,
                        data,
                        scale.integer_scale(),
                        Transform::Normal,
                    );
                }
                TraversalAction::DoChildren(origin)
            },
            |_, _, _| {},
            |_, _, _| true,
        );
    }
    pub fn feedback(
        &mut self,
        output: &Output,
        states: &RenderElementStates,
    ) -> OutputPresentationFeedback {
        self.desktop
            .presented
            .insert(output.clone(), states.clone());
        self.update_inhibitors();
        let mut feedback = OutputPresentationFeedback::new(output);
        for w in self.space.elements() {
            w.take_presentation_feedback(
                &mut feedback,
                |surface, data| {
                    states
                        .element_was_presented(surface)
                        .then(|| preferred(surface, data))
                        .flatten()
                },
                |s, _| surface_presentation_feedback_flags_from_states(s, states),
            );
        }
        feedback
    }
    pub fn send_frames(&self, output: &Output) {
        if self.desktop.lock.active {
            if let Some(s) = self.lock_frame_recipient(output) {
                smithay::desktop::utils::send_frames_surface_tree(
                    &s,
                    output,
                    self.start_time.elapsed(),
                    None,
                    |_, _| Some(output.clone()),
                );
            }
            return;
        }
        for layer in smithay::desktop::layer_map_for_output(output).layers() {
            layer.send_frame(output, self.start_time.elapsed(), None, |_, _| {
                Some(output.clone())
            });
        }
        for w in self.space.elements() {
            w.send_frame(
                output,
                self.start_time.elapsed(),
                Some(std::time::Duration::from_secs(1)),
                preferred,
            );
        }
    }
    pub fn advertise_dmabuf(
        &mut self,
        renderer: &GlesRenderer,
        device: Option<smithay::backend::drm::DrmNode>,
    ) {
        let formats = renderer.dmabuf_formats();
        if formats.iter().next().is_none() {
            return;
        }
        let dh = &self.display_handle;
        if let Some(node) = device.or_else(|| {
            EGLDevice::device_for_display(renderer.egl_context().display())
                .ok()
                .and_then(|d| d.try_get_render_node().ok().flatten())
        }) {
            if let Ok(feedback) = DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build()
            {
                self.dmabuf
                    .create_global_with_default_feedback::<Self>(dh, &feedback);
                return;
            }
        }
        self.dmabuf.create_global::<Self>(dh, formats);
    }
    pub fn import_buffers(&mut self, renderer: &mut GlesRenderer) {
        for (buffer, notifier) in self.imports.drain(..) {
            if renderer.import_dmabuf(&buffer, None).is_ok() {
                let _ = notifier.successful::<Self>();
            } else {
                notifier.failed();
            }
        }
    }
    pub fn animate(&mut self, dt: std::time::Duration) {
        if self.config.reduced_motion || self.desktop.reduce_motion {
            self.animation.settle_all();
        } else {
            self.animation.tick(dt);
        }
        if self.desktop.transition.as_ref().is_some_and(|t| {
            self.animation
                .spring(t.spring)
                .is_none_or(|s| s.is_settled())
        }) {
            if let Some(done) = self.desktop.transition.take() {
                self.animation.remove_spring(done.spring);
            }
        }
        crate::render::sync_spring_positions(self);
        self.wallpaper.cleanup(&mut self.animation);
    }
}
pub fn clamp_window(
    pos: Point<i32, Logical>,
    size: smithay::utils::Size<i32, Logical>,
    bounds: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    (
        pos.x
            .clamp(bounds.loc.x, bounds.loc.x + (bounds.size.w - size.w).max(0)),
        pos.y
            .clamp(bounds.loc.y, bounds.loc.y + (bounds.size.h - size.h).max(0)),
    )
        .into()
}
pub fn valid_scale(s: f64) -> bool {
    (0.5..=4.).contains(&s)
}
impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        if let Some(o) = self.space.output_under(self.cursor_location).next() {
            with_states(&surface, |s| {
                with_fractional_scale(s, |f| {
                    f.set_preferred_scale(o.current_scale().fractional_scale())
                })
            });
        }
        self.update_scales();
    }
}
impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf
    }
    fn dmabuf_imported(&mut self, _: &DmabufGlobal, buffer: Dmabuf, n: ImportNotifier) {
        self.imports.push((buffer, n));
    }
}
smithay::delegate_dmabuf!(State);
smithay::delegate_viewporter!(State);
smithay::delegate_fractional_scale!(State);
smithay::delegate_presentation!(State);

impl State {
    pub fn snap_target(
        &self,
        window: &Window,
        pos: Point<i32, Logical>,
        size: smithay::utils::Size<i32, Logical>,
    ) -> Point<i32, Logical> {
        let bounds = self.pointer_bounds();
        let mut neighbors = self.wm.neighbors_except(window, &self.space);
        for rect in &mut neighbors {
            rect.loc -= bounds.loc;
        }
        bounds.loc
            + crate::wm::snap(
                pos - bounds.loc,
                size,
                bounds.size,
                &neighbors,
                self.config.wm.attraction_radius,
                self.config.wm.outer_gap,
                self.config.wm.inner_gap,
            )
    }
}

/// Arm at most one deadline. Once a scene settles the loop sleeps on its file descriptors.
#[derive(Default)]
pub struct FrameWake {
    timer: std::rc::Rc<std::cell::Cell<Option<(std::time::Instant, calloop::RegistrationToken)>>>,
}
impl FrameWake {
    pub fn schedule(
        &mut self,
        handle: &calloop::LoopHandle<'static, State>,
        deadline: std::time::Instant,
    ) {
        if let Some((at, token)) = self.timer.get() {
            if at <= deadline {
                return;
            }
            handle.remove(token);
        }
        let timer = self.timer.clone();
        let token = handle
            .insert_source(
                calloop::timer::Timer::from_deadline(deadline),
                move |_, _, _| {
                    timer.set(None);
                    calloop::timer::TimeoutAction::Drop
                },
            )
            .expect("register frame timer");
        self.timer.set(Some((deadline, token)));
    }
}
pub fn rotation(degrees: u16) -> Option<Transform> {
    match degrees {
        0 => Some(Transform::Normal),
        90 => Some(Transform::_90),
        180 => Some(Transform::_180),
        270 => Some(Transform::_270),
        _ => None,
    }
}
