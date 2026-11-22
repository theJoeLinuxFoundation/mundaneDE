//! Root application — owns the sole top-level Entity. The sidebar is rendered
//! inline (it's stateless; selection lives on `App`). Each page is its own
//! Entity so caches survive selection changes.

use crate::pages::{Page, PagesView};
use crate::sidebar;
use crate::theme;
use gpui::InteractiveElement;
use gpui::{
    div, px, rgb, size, App as GpuiApp, AppContext as _, Bounds, Context, Entity, IntoElement,
    ParentElement, Render, Styled, Window, WindowBounds, WindowKind, WindowOptions,
};

pub struct App {
    pub selected: Page,
    navigation: [gpui::FocusHandle; 9],
    pub pages: Entity<PagesView>,
}

impl App {
    pub fn new(initial: Page, cx: &mut Context<Self>) -> Self {
        let pages = cx.new(|cx| PagesView::new(initial, cx));
        Self {
            selected: initial,
            navigation: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            pages,
        }
    }

    pub fn select(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.selected == page {
            return;
        }
        self.selected = page;
        let pages = self.pages.clone();
        pages.update(cx, |p, cx| p.set_active(page, cx));
        cx.notify();
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_entity = cx.entity();
        let prefs = theme::current();
        window.set_rem_size(px((prefs.points * (96. / 72.) * prefs.scale) as f32));
        if window.focused(cx).is_none() {
            window.focus(&self.navigation[self.selected as usize], cx);
        }

        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(theme::palette().surface))
            .text_color(rgb(theme::palette().text))
            .font_family(prefs.font)
            .on_key_down(|e: &gpui::KeyDownEvent, w, cx| {
                if e.keystroke.key == "tab" {
                    if e.keystroke.modifiers.shift {
                        w.focus_prev(cx)
                    } else {
                        w.focus_next(cx)
                    }
                    cx.stop_propagation();
                }
            })
            .child(sidebar::render(self.selected, app_entity, &self.navigation))
            .child(div().flex_1().min_w_0().h_full().child(self.pages.clone()))
    }
}

/// Open the main window and run the event loop.
pub fn run(initial: Page) {
    mundane_ui::run(move |cx: &mut GpuiApp| {
        theme::watch(cx);
        let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_min_size: Some(size(px(480.), px(320.))),
            kind: WindowKind::Normal,
            app_id: Some("mundane-settings".into()),
            ..Default::default()
        };
        cx.open_window(opts, move |_window, cx| cx.new(|cx| App::new(initial, cx)))
            .expect("open window");
        cx.activate(true);
    });
}
