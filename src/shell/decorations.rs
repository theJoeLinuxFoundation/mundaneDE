//! Title bars for windows that delegate decoration to the compositor.
use crate::state::State;
use cairo::{Context, Format, ImageSurface};
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    desktop::Window,
    reexports::wayland_protocols::xdg::{
        decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        shell::server::xdg_toplevel::State as Flag,
    },
    utils::{Logical, Point, Rectangle, Transform},
    wayland::shell::xdg::XdgShellHandler,
};
use std::cell::{Cell, RefCell};

pub const HEIGHT: i32 = 30;
#[derive(Default)]
struct Decoration {
    buffer: RefCell<Option<(String, i32, bool, MemoryRenderBuffer)>>,
    previous: Cell<Option<Rectangle<i32, Logical>>>,
}

pub fn decorated(w: &Window) -> bool {
    if let Some(t) = w.toplevel() {
        let s = t.current_state();
        return s.decoration_mode == Some(Mode::ServerSide) && !s.states.contains(Flag::Fullscreen);
    }
    w.x11_surface()
        .is_some_and(|x| !x.is_override_redirect() && !x.is_decorated() && !x.is_fullscreen())
}

pub fn draw(w: &Window, title: String, width: i32) -> Option<MemoryRenderBuffer> {
    let width = width.clamp(1, 16384);
    w.user_data().insert_if_missing(Decoration::default);
    let d = w.user_data().get::<Decoration>()?;
    let max = w
        .toplevel()
        .is_some_and(|t| t.current_state().states.contains(Flag::Maximized))
        || w.x11_surface().is_some_and(|x| x.is_maximized());
    let mut cache = d.buffer.borrow_mut();
    if let Some((last, size, m, buffer)) = cache.as_ref() {
        if last == &title && *size == width && *m == max {
            return Some(buffer.clone());
        }
    }
    let mut img = ImageSurface::create(Format::ARgb32, width, HEIGHT).ok()?;
    let c = Context::new(&img).ok()?;
    c.set_source_rgb(0.12, 0.15, 0.2);
    c.paint().ok()?;
    c.set_source_rgb(0.90, 0.93, 0.97);
    c.set_font_size(14.);
    c.save().ok()?;
    c.rectangle(
        7.,
        0.,
        (width - 3 * HEIGHT - 12).max(0) as f64,
        HEIGHT as f64,
    );
    c.clip();
    c.move_to(9., 20.);
    c.show_text(&title.replace('\0', "")).ok()?;
    c.restore().ok()?;
    for (i, glyph) in [if max { "▣" } else { "□" }, "×"].iter().enumerate() {
        c.move_to((width - (2 - i as i32) * HEIGHT + 9) as f64, 20.);
        c.show_text(glyph).ok()?;
    }
    c.move_to((width - 3 * HEIGHT + 10) as f64, 19.);
    c.rel_line_to(10., 0.);
    c.stroke().ok()?;
    drop(c);
    let buffer = MemoryRenderBuffer::from_slice(
        &img.data().ok()?,
        Fourcc::Argb8888,
        (width, HEIGHT),
        1,
        Transform::Normal,
        None,
    );
    *cache = Some((title, width, max, buffer.clone()));
    Some(buffer)
}
impl State {
    pub fn decoration_at(&self, p: Point<f64, Logical>) -> Option<(Window, i32)> {
        for w in self.space.elements().rev() {
            let Some(geo) = self.space.element_geometry(w) else {
                continue;
            };
            if w.surface_under(
                p - geo.loc.to_f64(),
                smithay::desktop::WindowSurfaceType::ALL,
            )
            .is_some()
            {
                return None;
            }
            if decorated(w) {
                let bar = Rectangle::new(
                    geo.loc - Point::from((0, HEIGHT)),
                    (geo.size.w, HEIGHT).into(),
                );
                if bar.to_f64().contains(p) {
                    return Some((w.clone(), (geo.loc.x + geo.size.w) - p.x as i32));
                }
            }
        }
        None
    }
    pub fn decoration_click(&mut self, w: Window, from_right: i32) {
        crate::focus::set_keyboard_focus(self, Some(&w));
        if from_right <= HEIGHT {
            if let Some(t) = w.toplevel() {
                t.send_close()
            } else if let Some(x) = w.x11_surface() {
                let _ = x.close();
            }
        } else if from_right <= 2 * HEIGHT {
            if let Some(t) = w.toplevel() {
                self.set_expanded(
                    t,
                    Flag::Maximized,
                    !t.current_state().states.contains(Flag::Maximized),
                );
            } else if let Some(x) = w.x11_surface() {
                let d = w.user_data().get::<Decoration>().unwrap();
                let max = !x.is_maximized();
                let mut g = self.work_area(&w);
                if max {
                    d.previous.set(self.space.element_geometry(&w));
                } else if let Some(old) = d.previous.take() {
                    g = old;
                }
                let _ = x.set_maximized(max);
                let _ = x.configure(g);
                self.space.map_element(w, g.loc, true);
            }
        } else if from_right <= 3 * HEIGHT {
            if let Some(t) = w.toplevel() {
                self.minimize_request(t.clone());
            } else {
                self.space.unmap_elem(&w);
                self.minimized.push(w);
                crate::focus::refocus(self);
            }
        } else if let Some(loc) = self.space.element_location(&w) {
            self.move_state = Some(crate::state::MoveState {
                window: w,
                offset: self.cursor_location - loc.to_f64(),
                client_grab: false,
                snap_preview: None,
            });
        }
        self.generation += 1;
    }
}
