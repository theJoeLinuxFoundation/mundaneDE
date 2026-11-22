//! Display page.

use crate::pages::{Page, PageState};
use crate::theme;
use gpui::{
    div, px, rgb, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window,
};
use mundane_settings_core::display::{self as backend, Mode, Monitor};

pub struct DisplayPage {
    state: PageState<Vec<Monitor>>,
    trial: bool,
    status: String,
    selections: std::collections::HashMap<String, (Mode, f32, u16, Option<[i32; 2]>)>,
    generation: u64,
}

impl DisplayPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = Self {
            state: PageState::Loading,
            trial: false,
            status: String::new(),
            selections: Default::default(),
            generation: 0,
        };
        s.spawn_refresh(cx);
        s
    }

    fn apply(&mut self, change: Option<(Monitor, Mode, f32)>, keep: bool, cx: &mut Context<Self>) {
        if matches!(self.state, PageState::Loading) {
            return;
        }
        self.state = PageState::Loading;
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |weak, cx| {
            let trying = change.is_some();
            let result = match change {
                Some((m, mode, scale)) => backend::trial(m, mode, scale).await,
                None => backend::finish_trial(keep).await,
            };
            let started = trying && result.is_ok();
            weak.update(cx, |this, cx| {
                this.trial = started;
                this.status = match result {
                    Ok(()) if trying => "Keep this layout within 15 seconds.".into(),
                    Ok(()) => String::new(),
                    Err(e) => e.to_string(),
                };
                this.selections.clear();
                this.spawn_refresh(cx);
                cx.notify();
            })
            .ok();
            if started {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(16))
                    .await;
                weak.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.trial = false;
                        this.status = "Previous layout restored.".into();
                        this.spawn_refresh(cx);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, async_cx| {
            let result = backend::query().await;
            weak.update(async_cx, |this, cx| {
                this.state = match result {
                    Ok(m) => PageState::Loaded {
                        data: m,
                        fetched_at: std::time::Instant::now(),
                    },
                    Err(e) => PageState::Error(e.to_string()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for DisplayPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.should_refresh(Page::Display.ttl()) {
            self.state = PageState::Loading;
            self.spawn_refresh(cx);
        }
        let body = match &self.state {
            PageState::Loaded { data, .. } if data.is_empty() => empty_view(),
            PageState::Loaded { data, .. } => {
                let mut col = div().flex().flex_col().gap_3();
                for m in data {
                    let mut card = monitor_card(m);
                    if let Some(current) = m
                        .current
                        .as_ref()
                        .filter(|_| backend::wayland() && !self.trial)
                    {
                        let (mode, scale, rotation, position) =
                            self.selections.get(&m.name).cloned().unwrap_or_else(|| {
                                (current.clone(), m.scale, m.rotation, m.position)
                            });
                        let mut positions = vec![m.position];
                        let logical = |mode: &Mode, scale: f32, rotation: u16| {
                            let (w, h) = (mode.width as f32 / scale, mode.height as f32 / scale);
                            if rotation % 180 == 0 {
                                [w.round() as i32, h.round() as i32]
                            } else {
                                [h.round() as i32, w.round() as i32]
                            }
                        };
                        let [width, height] = logical(&mode, scale, rotation);
                        for other in data.iter().filter(|o| o.name != m.name) {
                            if let Some(other_mode) = &other.current {
                                let [x, y] = other.origin;
                                let [w, h] = logical(other_mode, other.scale, other.rotation);
                                positions.extend(
                                    [[x - width, y], [x + w, y], [x, y - height], [x, y + h]]
                                        .map(Some),
                                );
                            }
                        }
                        let [x, y] = position.unwrap_or(m.origin);
                        let labels = [
                            format!("Mode: {}", mode.label()),
                            format!("Scale: {:.0}%", scale * 100.),
                            format!("Rotation: {rotation}°"),
                            format!("Position: {x}, {y}"),
                            "Test layout".into(),
                        ];
                        for (index, label) in labels.into_iter().enumerate() {
                            if index == 3 && positions.len() == 1 {
                                continue;
                            }
                            let mut output = m.clone();
                            output.rotation = rotation;
                            output.position = position;
                            let selected = mode.clone();
                            let positions = positions.clone();
                            card =
                                card.child(control(format!("{}-{index}", m.name), label).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let mut next =
                                            (selected.clone(), scale, rotation, position);
                                        match index {
                                            0 => {
                                                let modes = &output.modes;
                                                if let Some(i) =
                                                    modes.iter().position(|m| m == &selected)
                                                {
                                                    next.0 = modes[(i + 1) % modes.len()].clone();
                                                }
                                            }
                                            1 => {
                                                next.1 = [1., 1.25, 1.5, 1.75, 2.]
                                                    .into_iter()
                                                    .find(|s| *s > scale)
                                                    .unwrap_or(1.);
                                            }
                                            2 => next.2 = (rotation + 90) % 360,
                                            3 => {
                                                let i = positions
                                                    .iter()
                                                    .position(|p| *p == position)
                                                    .unwrap_or(0);
                                                next.3 = positions[(i + 1) % positions.len()];
                                            }
                                            _ => {
                                                this.apply(
                                                    Some((output.clone(), selected.clone(), scale)),
                                                    false,
                                                    cx,
                                                );
                                                return;
                                            }
                                        }
                                        this.selections.insert(output.name.clone(), next);
                                        cx.notify()
                                    }),
                                ));
                        }
                    }
                    col = col.child(card);
                }
                col
            }
            PageState::Error(e) => error_view(e),
            _ => skeleton(),
        };
        let mut content = div()
            .id("display-scroll")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .child(div().text_xl().pb_4().child("Display"))
            .child(body)
            .child(self.status.clone());
        if self.trial {
            for (label, keep) in [("Keep layout", true), ("Revert layout", false)] {
                content = content.child(
                    control(label.into(), label.into())
                        .on_click(cx.listener(move |this, _, _, cx| this.apply(None, keep, cx))),
                );
            }
        }
        content
    }
}

fn skeleton() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .h(px(140.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
        .child(
            div()
                .h(px(140.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
}

fn empty_view() -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(200.))
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .text_color(rgb(theme::palette().secondary))
        .child("No connected displays.")
}

fn error_view(msg: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(200.))
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .text_color(rgb(theme::palette().secondary))
        .child(format!("Couldn't query displays: {msg}"))
}

fn control(id: String, label: String) -> gpui::Stateful<gpui::Div> {
    use mundane_ui::ControlRole;
    gpui::Role::Button
        .control(gpui::SharedString::from(id), label.clone())
        .tab_index(0)
        .border_1()
        .border_color(rgb(theme::palette().border))
        .focus(|s| s.border_color(rgb(theme::palette().focus)))
        .cursor_pointer()
        .px_3()
        .py_1()
        .child(label)
}

fn monitor_card(m: &Monitor) -> gpui::Div {
    let cur = m
        .current
        .as_ref()
        .map(|c| c.label())
        .unwrap_or_else(|| "off".into());
    div()
        .flex()
        .flex_col()
        .gap_2()
        .px_4()
        .py_3()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(div().text_color(rgb(theme::palette().text)).child(format!(
                    "{}{}",
                    m.name,
                    if m.primary { " (primary)" } else { "" }
                )))
                .child(
                    div().text_color(rgb(theme::palette().secondary)).child(
                        if m.connected {
                            "connected"
                        } else {
                            "disconnected"
                        }
                        .to_string(),
                    ),
                ),
        )
        .child(
            div()
                .text_color(rgb(theme::palette().secondary))
                .child(format!("Current: {cur}  •  Scale: {:.2}", m.scale)),
        )
        .child(
            div()
                .text_color(rgb(theme::palette().secondary))
                .child(format!("{} mode(s) available", m.modes.len())),
        )
}
