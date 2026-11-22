//! Settings navigation uses the same active panel button as desktop chrome.
use crate::app::App;
use crate::pages::Page;
use crate::theme;
use gpui::{
    div, px, rgb, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};

const SIDEBAR_WIDTH: f32 = 220.0;
const ROW_HEIGHT: f32 = 44.0;
const TOP_PAD: f32 = 16.0;
const HEADER_HEIGHT: f32 = 48.0;

pub fn render(selected: Page, app: Entity<App>, nav: &[gpui::FocusHandle; 9]) -> impl IntoElement {
    div()
        .id("settings-navigation")
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .bg(rgb(theme::palette().panel))
        .pt(px(TOP_PAD))
        .child(
            div()
                .h(px(HEADER_HEIGHT))
                .px_4()
                .text_color(rgb(theme::palette().focus))
                .child("mundaneDE"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .px(px(8.))
                .child(row(Page::Compositor, selected, app.clone(), nav))
                .child(row(Page::Appearance, selected, app.clone(), nav))
                .child(row(Page::Display, selected, app.clone(), nav))
                .child(row(Page::Sound, selected, app.clone(), nav))
                .child(row(Page::Network, selected, app.clone(), nav))
                .child(row(Page::Users, selected, app.clone(), nav))
                .child(row(Page::Software, selected, app.clone(), nav))
                .child(row(Page::DateTime, selected, app.clone(), nav))
                .child(row(Page::System, selected, app, nav)),
        )
}

fn row(
    page: Page,
    selected: Page,
    app: Entity<App>,
    nav: &[gpui::FocusHandle; 9],
) -> impl IntoElement {
    let content = div()
        .flex()
        .items_center()
        .gap(mundane_ui::tokens::space::MD)
        .child(div().w(mundane_ui::tokens::icon::LARGE).child(page.icon()))
        .child(page.label());
    mundane_ui::panel_button(
        page.label(),
        content,
        &nav[page as usize],
        mundane_ui::PanelState {
            active: page == selected,
            ..Default::default()
        },
        theme::palette(),
        move |_, cx| {
            app.update(cx, |app, cx| app.select(page, cx));
        },
    )
    .h(px(ROW_HEIGHT))
    .justify_start()
}
