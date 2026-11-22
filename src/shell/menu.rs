//! The xdg-toplevel system menu. Its rows can be activated by pointer or keyboard.
use crate::state::State;
use cairo::{Context, Format, ImageSurface};
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    input::keyboard::keysyms,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as Flag,
    utils::{Logical, Point, Size, Transform},
    wayland::shell::xdg::{ToplevelSurface, XdgShellHandler},
};

const WIDTH: i32 = 160;
const ROW: i32 = 32;
const ROWS: i32 = 4;

pub struct WindowMenu {
    surface: ToplevelSurface,
    pub origin: Point<i32, Logical>,
    pub buffer: MemoryRenderBuffer,
    selected: usize,
}

impl WindowMenu {
    pub fn new(
        surface: ToplevelSurface,
        at: Point<i32, Logical>,
        output: Size<i32, Logical>,
    ) -> anyhow::Result<Self> {
        let origin = (
            at.x.clamp(0, (output.w - WIDTH).max(0)),
            at.y.clamp(0, (output.h - ROWS * ROW).max(0)),
        )
            .into();
        let buffer = Self::draw(&surface, 0)?;
        Ok(Self {
            surface,
            origin,
            buffer,
            selected: 0,
        })
    }

    fn draw(surface: &ToplevelSurface, selected: usize) -> anyhow::Result<MemoryRenderBuffer> {
        let mut image = ImageSurface::create(Format::ARgb32, WIDTH, ROW * ROWS)?;
        let ctx = Context::new(&image)?;
        ctx.set_source_rgb(0.13, 0.13, 0.15);
        ctx.paint()?;
        ctx.set_source_rgb(0.22, 0.33, 0.49);
        ctx.rectangle(0., (selected as i32 * ROW).into(), WIDTH.into(), ROW.into());
        ctx.fill()?;
        let flags = surface.current_state().states;
        let maximize = if flags.contains(Flag::Maximized) {
            "Restore"
        } else {
            "Maximize"
        };
        let fullscreen = if flags.contains(Flag::Fullscreen) {
            "Leave fullscreen"
        } else {
            "Fullscreen"
        };
        ctx.set_font_size(15.);
        ctx.set_source_rgb(1., 1., 1.);
        for (i, label) in ["Minimize", maximize, fullscreen, "Close"]
            .iter()
            .enumerate()
        {
            ctx.move_to(12., (i as i32 * ROW + 21).into());
            ctx.show_text(label)?;
        }
        drop(ctx);
        let bytes = image.data()?;
        Ok(MemoryRenderBuffer::from_slice(
            &bytes,
            Fourcc::Argb8888,
            (WIDTH, ROW * ROWS),
            1,
            Transform::Normal,
            None,
        ))
    }

    fn select(&mut self, index: usize) {
        if self.selected != index {
            if let Ok(buffer) = Self::draw(&self.surface, index) {
                self.selected = index;
                self.buffer = buffer;
            }
        }
    }

    pub fn row(&self, pos: Point<f64, Logical>) -> Option<usize> {
        let local = pos - self.origin.to_f64();
        ((0.0..WIDTH.into()).contains(&local.x) && (0.0..(ROW * ROWS).into()).contains(&local.y))
            .then_some((local.y as usize) / ROW as usize)
    }
    pub fn hover(&mut self, pos: Point<f64, Logical>) {
        if let Some(row) = self.row(pos) {
            self.select(row);
        }
    }
}

impl State {
    pub fn menu_key(&mut self, key: u32) {
        let Some(menu) = self.window_menu.as_mut() else {
            return;
        };
        match key {
            keysyms::KEY_Escape => self.window_menu = None,
            keysyms::KEY_Up => menu.select((menu.selected + ROWS as usize - 1) % ROWS as usize),
            keysyms::KEY_Down => menu.select((menu.selected + 1) % ROWS as usize),
            keysyms::KEY_Return | keysyms::KEY_space => {
                let row = menu.selected;
                self.menu_action(Some(row));
            }
            _ => {}
        }
    }
    pub fn menu_action(&mut self, row: Option<usize>) {
        let Some(menu) = self.window_menu.take() else {
            return;
        };
        let t = menu.surface;
        match row {
            Some(0) => self.minimize_request(t),
            Some(1) => self.set_expanded(
                &t,
                Flag::Maximized,
                !t.current_state().states.contains(Flag::Maximized),
            ),
            Some(2) => self.set_expanded(
                &t,
                Flag::Fullscreen,
                !t.current_state().states.contains(Flag::Fullscreen),
            ),
            Some(3) => t.send_close(),
            _ => {}
        }
    }
    pub fn dismiss_menu_for(&mut self, t: &ToplevelSurface) {
        if self.window_menu.as_ref().is_some_and(|m| m.surface == *t) {
            self.window_menu = None;
        }
    }
}
