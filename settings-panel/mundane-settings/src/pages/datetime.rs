//! Date & time page — timezone picker + NTP toggle.

use crate::pages::{Page, PageState};
use crate::theme;
use gpui::{
    div, px, rgb, AnyElement, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Window,
};
use mundane_settings_core::datetime::{self as backend, DateTimeStatus};

pub struct DateTimePage {
    status: PageState<DateTimeStatus>,
    tz_list: Vec<String>,
    tz_list_loaded: bool,
    tz_open: bool,
    in_flight: bool,
}

impl DateTimePage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = Self {
            status: PageState::Empty,
            tz_list: Vec::new(),
            tz_list_loaded: false,
            tz_open: false,
            in_flight: false,
        };
        s.spawn_refresh(cx);
        s
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, async_cx| {
            let status = backend::status().await;
            let tz = backend::list_timezones().await.unwrap_or_default();
            weak.update(async_cx, |this, cx| {
                this.status = match status {
                    Ok(s) => PageState::Loaded {
                        data: s,
                        fetched_at: std::time::Instant::now(),
                    },
                    Err(e) => PageState::Error(e.to_string()),
                };
                this.tz_list = tz;
                this.tz_list_loaded = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_tz_list(&mut self, cx: &mut Context<Self>) {
        self.tz_open = !self.tz_open;
        cx.notify();
    }

    fn set_tz(&mut self, tz: String, cx: &mut Context<Self>) {
        if self.in_flight {
            return;
        }
        if let PageState::Loaded { data, .. } = &mut self.status {
            data.timezone = tz.clone();
        }
        self.in_flight = true;
        self.tz_open = false;
        cx.notify();
        cx.spawn(async move |weak, async_cx| {
            let _ = backend::set_timezone(&tz).await;
            weak.update(async_cx, |this, cx| {
                this.in_flight = false;
                cx.notify();
                this.spawn_refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn toggle_ntp(&mut self, cx: &mut Context<Self>) {
        if self.in_flight {
            return;
        }
        let new_state = match &self.status {
            PageState::Loaded { data, .. } => !data.ntp_enabled,
            _ => return,
        };
        if let PageState::Loaded { data, .. } = &mut self.status {
            data.ntp_enabled = new_state;
        }
        self.in_flight = true;
        cx.notify();
        cx.spawn(async move |weak, async_cx| {
            let _ = backend::set_ntp(new_state).await;
            weak.update(async_cx, |this, cx| {
                this.in_flight = false;
                cx.notify();
                this.spawn_refresh(cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for DateTimePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.status.should_refresh(Page::DateTime.ttl()) {
            self.spawn_refresh(cx);
        }
        let body: AnyElement = match &self.status {
            PageState::Loaded { data, .. } => loaded_view(
                window,
                data,
                &self.tz_list,
                self.tz_open,
                self.in_flight,
                cx,
            )
            .into_any_element(),
            PageState::Error(e) => error_view(e).into_any_element(),
            _ => skeleton().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .child(div().text_xl().pb_4().child("Date & Time"))
            .child(body)
    }
}

fn skeleton() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .h(px(56.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
        .child(
            div()
                .h(px(56.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
}

fn error_view(msg: &str) -> gpui::Div {
    div()
        .h(px(120.))
        .px_4()
        .flex()
        .items_center()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .text_color(rgb(theme::palette().secondary))
        .child(format!("timedatectl: {msg}"))
}

fn loaded_view(
    window: &mut Window,
    d: &DateTimeStatus,
    tz_list: &[String],
    tz_open: bool,
    in_flight: bool,
    cx: &mut Context<DateTimePage>,
) -> gpui::Div {
    let mut col = div().flex().flex_col().gap_3();
    col = col.child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .py_3()
            .bg(rgb(theme::palette().panel))
            .rounded(px(10.))
            .child(
                div()
                    .text_color(rgb(theme::palette().secondary))
                    .child("Current time"),
            )
            .child(
                div()
                    .text_color(rgb(theme::palette().text))
                    .child(d.local_time.clone()),
            ),
    );
    col = col.child(tz_row(window, &d.timezone, tz_open, in_flight, cx));
    if tz_open {
        col = col.child(tz_list_view(window, tz_list, &d.timezone, cx));
    }
    col = col.child(ntp_row(window, d.ntp_enabled, in_flight, cx));
    col
}

fn tz_row(
    window: &mut Window,
    current: &str,
    open: bool,
    in_flight: bool,
    cx: &mut Context<DateTimePage>,
) -> AnyElement {
    let chevron = if open { "▾" } else { "▸" };
    let mut row = crate::widgets::action("tz-row", "Timezone", window, cx, |this, cx| {
        this.toggle_tz_list(cx)
    })
    .flex()
    .flex_row()
    .items_center()
    .justify_between()
    .child(
        div()
            .text_color(rgb(theme::palette().secondary))
            .child("Timezone"),
    )
    .child(
        div()
            .flex()
            .flex_row()
            .gap_3()
            .child(
                div()
                    .text_color(rgb(theme::palette().text))
                    .child(current.to_string()),
            )
            .child(
                div()
                    .text_color(rgb(theme::palette().secondary))
                    .child(chevron.to_string()),
            ),
    );
    if in_flight {
        row = row.border_b_2().border_color(rgb(theme::palette().focus));
    }
    row.into_any_element()
}

fn tz_list_view(
    window: &mut Window,
    tz_list: &[String],
    current: &str,
    cx: &mut Context<DateTimePage>,
) -> gpui::Div {
    let mut col = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .py_2()
        .bg(rgb(theme::palette().surface))
        .rounded(px(10.));
    if tz_list.is_empty() {
        col = col.child(
            div()
                .px_4()
                .py_2()
                .text_color(rgb(theme::palette().secondary))
                .child("Loading timezones…"),
        );
        return col;
    }
    // Limit to the user's region prefix for compactness — the full IANA list is
    // 500+ entries which would dominate the page. We show the entries that
    // share the current timezone's continent first, then the rest.
    let continent_prefix = current.split_once('/').map(|(p, _)| p).unwrap_or("");
    let (same, other): (Vec<&String>, Vec<&String>) = tz_list
        .iter()
        .partition(|tz| !continent_prefix.is_empty() && tz.starts_with(continent_prefix));
    let limited: Vec<&String> = same.into_iter().chain(other).take(40).collect();
    for tz in limited {
        let is_current = tz == current;
        let tz_owned = tz.clone();
        let row = crate::widgets::action(
            SharedString::from(format!("tz-opt:{tz}")),
            tz.clone(),
            window,
            cx,
            move |this, cx| this.set_tz(tz_owned.clone(), cx),
        )
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_color(rgb(theme::palette().text))
                .child(tz.clone()),
        )
        .child(
            div()
                .text_color(rgb(if is_current {
                    theme::palette().focus
                } else {
                    theme::palette().secondary
                }))
                .child(if is_current { "Current" } else { "" }.to_string()),
        );
        col = col.child(row);
    }
    col
}

fn ntp_row(
    window: &mut Window,
    enabled: bool,
    in_flight: bool,
    cx: &mut Context<DateTimePage>,
) -> AnyElement {
    let focus = crate::widgets::focus("ntp-row", window, cx);
    let view = cx.weak_entity();
    mundane_ui::labelled_switch(
        "Sync time over the network",
        &focus,
        enabled,
        !in_flight,
        theme::palette(),
        move |_, _, cx| {
            let _ = view.update(cx, |this, cx| this.toggle_ntp(cx));
        },
    )
    .into_any_element()
}
