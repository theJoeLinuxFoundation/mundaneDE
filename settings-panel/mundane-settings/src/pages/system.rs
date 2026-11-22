//! System page.

use crate::pages::{Page, PageState};
use crate::theme;
use gpui::{
    div, px, rgb, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window,
};
use mundane_settings_core::{
    os_release::{self, OsRelease},
    system::{self as sys, LogLine},
};

#[derive(Default, Clone)]
pub struct SystemData {
    pub os: OsRelease,
    pub hostname: String,
}

pub struct SystemPage {
    state: PageState<SystemData>,
    log: Vec<LogLine>,
    upgrading: bool,
}

impl SystemPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = Self {
            state: PageState::Empty,
            log: Vec::new(),
            upgrading: false,
        };
        s.spawn_refresh(cx);
        s
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, async_cx| {
            let os = os_release::load().await.unwrap_or_default();
            let hostname = os_release::hostname().await.unwrap_or_default();
            weak.update(async_cx, |this, cx| {
                this.state = PageState::Loaded {
                    data: SystemData { os, hostname },
                    fetched_at: std::time::Instant::now(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_upgrade(&mut self, cx: &mut Context<Self>) {
        if self.upgrading {
            return;
        }
        self.upgrading = true;
        self.log.clear();
        cx.notify();
        cx.spawn(async move |weak, async_cx| {
            let mut rx = match sys::run_upgrade().await {
                Ok(rx) => rx,
                Err(e) => {
                    weak.update(async_cx, |this, cx| {
                        this.log.push(LogLine::Stderr(e.to_string()));
                        this.upgrading = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            while let Some(line) = rx.recv().await {
                let is_exit = matches!(line, LogLine::Exit(_));
                weak.update(async_cx, |this, cx| {
                    this.log.push(line.clone());
                    if is_exit {
                        this.upgrading = false;
                    }
                    cx.notify();
                })
                .ok();
                if is_exit {
                    break;
                }
            }
        })
        .detach();
    }
}

impl Render for SystemPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.should_refresh(Page::System.ttl()) {
            self.spawn_refresh(cx);
        }

        let upgrading = self.upgrading;
        let label = if matches!(&self.state,PageState::Loaded {data,..} if data.os.id.as_deref()==Some("joe"))
        {
            "Open Packages"
        } else {
            "Check for Updates"
        };
        let upgrade_button = div()
            .id("check-updates")
            .role(gpui::Role::Button)
            .aria_label(label)
            .tab_index(0)
            .focus(|s| s.border_2().border_color(rgb(theme::palette().focus)))
            .px_4()
            .py_2()
            .bg(rgb(if upgrading {
                theme::palette().panel
            } else {
                theme::palette().focus
            }))
            .text_color(rgb(theme::palette().surface))
            .rounded(px(8.))
            .cursor_pointer()
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.start_upgrade(cx);
            }))
            .child(if upgrading { "Updating…" } else { label }.to_string());

        let body = match &self.state {
            PageState::Loaded { data, .. } => loaded_view(data, &self.log, upgrade_button),
            _ => skeleton(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .child(div().text_xl().pb_4().child("System"))
            .child(body)
    }
}

fn skeleton() -> gpui::Div {
    div()
        .h(px(220.))
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
}

fn loaded_view(d: &SystemData, log: &[LogLine], upgrade_button: impl IntoElement) -> gpui::Div {
    let pretty =
        d.os.pretty_name
            .clone()
            .unwrap_or_else(|| "mundaneDE".into());
    let version = d.os.version.clone().unwrap_or_default();
    let hostname_row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .child(
            div()
                .text_color(rgb(theme::palette().secondary))
                .child("Hostname"),
        )
        .child(
            div()
                .text_color(rgb(theme::palette().text))
                .child(d.hostname.clone()),
        );
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_4()
                .py_3()
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.))
                .child(div().text_color(rgb(theme::palette().text)).child(pretty))
                .child(
                    div()
                        .text_color(rgb(theme::palette().secondary))
                        .child(version),
                ),
        )
        .child(hostname_row)
        .child(upgrade_button)
        .child(log_view(log))
}

fn log_view(log: &[LogLine]) -> gpui::Div {
    let mut pre = div()
        .flex()
        .flex_col()
        .min_h(px(120.))
        .px_3()
        .py_2()
        .bg(rgb(theme::palette().surface))
        .rounded(px(8.))
        .text_color(rgb(theme::palette().secondary));
    for line in log.iter().rev().take(200).rev() {
        let (text, color) = match line {
            LogLine::Stdout(s) => (s.clone(), theme::palette().text),
            LogLine::Stderr(s) => (s.clone(), theme::palette().secondary),
            LogLine::Exit(c) => (format!("[exit {c}]"), theme::palette().focus),
        };
        pre = pre.child(div().text_color(rgb(color)).child(text));
    }
    pre
}
