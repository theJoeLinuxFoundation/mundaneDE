use super::{Pending, Source, State};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
            gles::{GlesRenderbuffer, GlesRenderer},
            Bind, Color32F, ExportMem, Offscreen,
        },
    },
    reexports::wayland_server::protocol::wl_shm,
    utils::{Rectangle, Transform},
    wayland::shm::with_buffer_contents_mut,
};

impl State {
    pub fn copy_capture_frames(&mut self, renderer: &mut GlesRenderer) {
        if self.capture.pending.is_empty() {
            return;
        }
        for p in std::mem::take(&mut self.capture.pending) {
            if !p.frame.alive() {
                continue;
            }
            let Some(size) = self.capture_size(&p.source) else {
                p.frame.fail();
                continue;
            };
            if self.desktop.lock.active {
                p.frame.fail();
                continue;
            }
            if p.frame.resized(size) {
                continue;
            }
            // A consumer can discard a preroll buffer when PipeWire enters streaming.
            // Refresh outstanding requests on a static scene so a newly playing client
            // receives a frame even if the user never moves the pointer.
            if let Some((generation, deadline)) = p.after {
                if generation == self.generation && std::time::Instant::now() < deadline {
                    self.capture.wake.schedule(&self.handle, deadline);
                    self.capture.pending.push(p);
                    continue;
                }
            }
            match self.capture_frame(renderer, &p) {
                Ok(()) => p.frame.ready(
                    &p.buffer,
                    self.generation,
                    self.clock.now().into(),
                    p.region.size,
                ),
                Err(e) => {
                    tracing::warn!(%e,"capture frame failed");
                    p.frame.fail();
                }
            }
        }
    }
    fn capture_frame(&mut self, renderer: &mut GlesRenderer, p: &Pending) -> anyhow::Result<()> {
        let size = self
            .capture_size(&p.source)
            .ok_or_else(|| anyhow::anyhow!("source gone"))?;
        anyhow::ensure!(
            Rectangle::from_size(size).contains_rect(p.region),
            "source resized"
        );
        let (elements, clear) = match &p.source {
            Source::Output(name) => {
                let output = self.capture_output(name).unwrap();
                crate::render::collect_elements(renderer, self, &output, p.cursor)
            }
            Source::Window(id) => {
                let w = self.capture_window(id).unwrap();
                let origin = smithay::utils::Point::from((0, 0)) - w.geometry().loc;
                let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    w.render_elements(renderer, origin.to_physical(1), 1.0.into(), 1.0);
                let mut elements = Vec::new();
                let pointer = self.cursor_location;
                if p.cursor
                    && self.hit_layer(pointer, true).is_none()
                    && self
                        .space
                        .element_under(pointer)
                        .is_some_and(|(top, _)| top == &w)
                {
                    if let Some(geometry) = self.space.element_geometry(&w) {
                        elements.extend(crate::render::cursor_elements(
                            renderer,
                            self,
                            pointer - geometry.loc.to_f64(),
                            1.0,
                        ));
                    }
                }
                elements.extend(
                    surfaces
                        .into_iter()
                        .map(crate::render::MundaneRenderElement::from),
                );
                (elements, Color32F::BLACK)
            }
        };
        let mut target: GlesRenderbuffer =
            renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())?;
        let mut fb = renderer.bind(&mut target)?;
        let scale = match &p.source {
            Source::Output(n) => self
                .capture_output(n)
                .unwrap()
                .current_scale()
                .fractional_scale(),
            Source::Window(_) => 1.0,
        };
        let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
        tracker.render_output(renderer, &mut fb, 0, &elements, clear)?;
        let r = p.region;
        let mapping = renderer.copy_framebuffer(
            &fb,
            Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into()),
            Fourcc::Argb8888,
        )?;
        let pixels = renderer.map_texture(&mapping)?;
        let stride = r.size.w as usize * 4;
        let rows = r.size.h as usize;
        with_buffer_contents_mut(&p.buffer, |ptr, len, b| -> anyhow::Result<()> {
            anyhow::ensure!(
                b.format == wl_shm::Format::Xrgb8888
                    && b.width == r.size.w
                    && b.height == r.size.h
                    && b.stride >= r.size.w * 4
                    && b.offset >= 0,
                "incompatible SHM buffer"
            );
            let offset = b.offset as usize;
            let pitch = b.stride as usize;
            anyhow::ensure!(
                offset + (rows - 1) * pitch + stride <= len && pixels.len() >= rows * stride,
                "short mapping"
            );
            for row in 0..rows {
                // The pool belongs to the client. Never form a Rust reference to its memory.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        pixels.as_ptr().add(row * stride),
                        ptr.add(offset + row * pitch),
                        stride,
                    );
                }
            }
            Ok(())
        })??;
        Ok(())
    }
}
