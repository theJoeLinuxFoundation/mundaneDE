//! Compositor page — the page that exists for mundaneDE specifically.
//!
//! Every control writes through to `~/.config/mundane/config.toml` via
//! `mundane_settings_core::compositor::update`. The mundaneDE compositor
//! watches that file and applies the diff live — change a slider here, see the
//! result in the next frame.

use crate::pages::{Page, PageState};
use crate::theme;
use mundane_settings_core::compositor as backend;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

/// Snapshot of the on-disk config the page renders against. The "current" values are
/// updated optimistically on every step, then synced from disk after a successful write.
#[derive(Debug, Clone)]
pub struct CompositorData {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
    pub attraction_radius: i32,
    pub outer_gap: i32,
    pub inner_gap: i32,
    pub wallpaper_mode: String,
}

pub struct CompositorPage {
    state: PageState<CompositorData>,
    in_flight: u32,
    last_error: Option<String>,
}

/// What changed on this step — kept as a tagged enum so the write closure for `update`
/// can stay small and `Send + 'static`.
enum Edit {
    Stiffness(f64),
    Damping(f64),
    Mass(f64),
    Radius(i32),
    OuterGap(i32),
    InnerGap(i32),
    WallpaperMode(String),
}

const MODES: &[&str] = &["fill", "fit", "stretch", "center", "tile"];

impl CompositorPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = Self { state: PageState::Empty, in_flight: 0, last_error: None };
        s.spawn_refresh(cx);
        s
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, async_cx| {
            let result = backend::load().await;
            weak.update(async_cx, |this, cx| {
                this.state = match result {
                    Ok(f) => {
                        let (stiffness, damping, mass) = f.spring();
                        let (radius, outer, inner) = f.wm();
                        let mode = f.wallpaper_mode();
                        PageState::Loaded {
                            data: CompositorData {
                                stiffness,
                                damping,
                                mass,
                                attraction_radius: radius,
                                outer_gap: outer,
                                inner_gap: inner,
                                wallpaper_mode: mode,
                            },
                            fetched_at: std::time::Instant::now(),
                        }
                    }
                    Err(e) => PageState::Error(e.to_string()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Apply an edit optimistically, then persist. On failure, revert and surface the error.
    fn apply(&mut self, edit: Edit, cx: &mut Context<Self>) {
        let prev = match &self.state {
            PageState::Loaded { data, .. } => data.clone(),
            _ => return,
        };
        if let PageState::Loaded { data, .. } = &mut self.state {
            match &edit {
                Edit::Stiffness(v) => data.stiffness = *v,
                Edit::Damping(v) => data.damping = *v,
                Edit::Mass(v) => data.mass = *v,
                Edit::Radius(v) => data.attraction_radius = *v,
                Edit::OuterGap(v) => data.outer_gap = *v,
                Edit::InnerGap(v) => data.inner_gap = *v,
                Edit::WallpaperMode(s) => data.wallpaper_mode = s.clone(),
            }
        }
        self.in_flight += 1;
        self.last_error = None;
        cx.notify();

        let mutate_edit = clone_edit(&edit);
        let snapshot = self.snapshot();
        cx.spawn(async move |weak, async_cx| {
            let result = backend::update(move |f| match mutate_edit {
                Edit::Stiffness(_) | Edit::Damping(_) | Edit::Mass(_) => {
                    f.set_spring(snapshot.stiffness, snapshot.damping, snapshot.mass);
                }
                Edit::Radius(_) | Edit::OuterGap(_) | Edit::InnerGap(_) => {
                    f.set_wm(
                        snapshot.attraction_radius,
                        snapshot.outer_gap,
                        snapshot.inner_gap,
                    );
                }
                Edit::WallpaperMode(ref m) => {
                    f.set_wallpaper_mode(m);
                }
            })
            .await;
            weak.update(async_cx, move |this, cx| {
                this.in_flight = this.in_flight.saturating_sub(1);
                match result {
                    Ok(()) => {}
                    Err(e) => {
                        // Revert.
                        if let PageState::Loaded { data, .. } = &mut this.state {
                            *data = prev;
                        }
                        this.last_error = Some(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn snapshot(&self) -> CompositorData {
        match &self.state {
            PageState::Loaded { data, .. } => data.clone(),
            _ => CompositorData {
                stiffness: 100.0,
                damping: 10.0,
                mass: 1.0,
                attraction_radius: 24,
                outer_gap: 8,
                inner_gap: 8,
                wallpaper_mode: "fill".into(),
            },
        }
    }
}

fn clone_edit(e: &Edit) -> Edit {
    match e {
        Edit::Stiffness(v) => Edit::Stiffness(*v),
        Edit::Damping(v) => Edit::Damping(*v),
        Edit::Mass(v) => Edit::Mass(*v),
        Edit::Radius(v) => Edit::Radius(*v),
        Edit::OuterGap(v) => Edit::OuterGap(*v),
        Edit::InnerGap(v) => Edit::InnerGap(*v),
        Edit::WallpaperMode(s) => Edit::WallpaperMode(s.clone()),
    }
}

/// Damping ratio ζ = damping / (2 * sqrt(stiffness * mass)).
fn zeta(stiffness: f64, damping: f64, mass: f64) -> f64 {
    let denom = 2.0 * (stiffness * mass).sqrt();
    if denom <= 0.0 { 0.0 } else { damping / denom }
}

impl Render for CompositorPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.should_refresh(Page::Compositor.ttl()) {
            self.spawn_refresh(cx);
        }
        let in_flight = self.in_flight > 0;
        let last_error = self.last_error.clone();
        let body = match &self.state {
            PageState::Loaded { data, .. } => data_view(data.clone(), in_flight, cx),
            _ => skeleton(),
        };
        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .child(div().text_xl().pb_4().child("Compositor"))
            .child(body);
        if let Some(err) = last_error {
            root = root.child(
                div()
                    .mt_3()
                    .px_4()
                    .py_2()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(format!("Couldn't save: {err}")),
            );
        }
        root
    }
}

fn skeleton() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(skel_row("Spring stiffness"))
        .child(skel_row("Spring damping"))
        .child(skel_row("Spring mass"))
        .child(skel_row("Attraction radius"))
        .child(skel_row("Outer gap"))
        .child(skel_row("Inner gap"))
        .child(skel_row("Wallpaper scaling"))
}

fn skel_row(label: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(56.))
        .px_4()
        .bg(rgb(theme::SURFACE))
        .rounded(px(10.))
        .child(div().text_color(rgb(theme::TEXT_MUTED)).child(label))
        .child(div().w(px(180.)).h(px(28.)).bg(rgb(theme::SIDEBAR)).rounded(px(6.)))
}

fn data_view(d: CompositorData, in_flight: bool, cx: &mut Context<CompositorPage>) -> gpui::Div {
    let mut col = div().flex().flex_col().gap_3();
    col = col.child(section_header("Spring physics"));
    col = col.child(stepper_row_f64(
        "Stiffness",
        d.stiffness,
        10.0,
        1.0,
        500.0,
        in_flight,
        |v| Edit::Stiffness(v),
        cx,
    ));
    col = col.child(stepper_row_f64(
        "Damping",
        d.damping,
        1.0,
        0.0,
        60.0,
        in_flight,
        |v| Edit::Damping(v),
        cx,
    ));
    col = col.child(stepper_row_f64(
        "Mass",
        d.mass,
        0.1,
        0.1,
        10.0,
        in_flight,
        |v| Edit::Mass(v),
        cx,
    ));
    col = col.child(
        div()
            .px_4()
            .py_2()
            .text_color(rgb(theme::TEXT_MUTED))
            .child(format!(
                "ζ (damping ratio) = {:.2}",
                zeta(d.stiffness, d.damping, d.mass)
            )),
    );

    col = col.child(section_header("Magnetic window manager"));
    col = col.child(stepper_row_i32(
        "Attraction radius (px)",
        d.attraction_radius,
        4,
        0,
        96,
        in_flight,
        |v| Edit::Radius(v),
        cx,
    ));
    col = col.child(stepper_row_i32(
        "Outer gap (px)",
        d.outer_gap,
        2,
        0,
        32,
        in_flight,
        |v| Edit::OuterGap(v),
        cx,
    ));
    col = col.child(stepper_row_i32(
        "Inner gap (px)",
        d.inner_gap,
        2,
        0,
        32,
        in_flight,
        |v| Edit::InnerGap(v),
        cx,
    ));

    col = col.child(section_header("Wallpaper"));
    col = col.child(mode_picker_row(&d.wallpaper_mode, in_flight, cx));

    col
}

fn section_header(label: &'static str) -> gpui::Div {
    div()
        .mt_2()
        .text_color(rgb(theme::TEXT_MUTED))
        .child(label)
}

fn stepper_button_style() -> gpui::Div {
    div()
        .w(px(32.))
        .h(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(theme::SIDEBAR))
        .text_color(rgb(theme::TEXT))
        .rounded(px(6.))
        .cursor_pointer()
}

fn stepper_row_f64(
    label: &'static str,
    value: f64,
    step: f64,
    min: f64,
    max: f64,
    in_flight: bool,
    edit: impl Fn(f64) -> Edit + Copy + 'static,
    cx: &mut Context<CompositorPage>,
) -> gpui::Div {
    let down_v = (value - step).max(min);
    let up_v = (value + step).min(max);
    let down_btn = stepper_button_style()
        .id(SharedString::from(format!("dn:{label}")))
        .hover(|s| s.bg(rgb(theme::BG)))
        .on_click(cx.listener(move |this, _e, _w, cx| this.apply(edit(down_v), cx)))
        .child("−");
    let up_btn = stepper_button_style()
        .id(SharedString::from(format!("up:{label}")))
        .hover(|s| s.bg(rgb(theme::BG)))
        .on_click(cx.listener(move |this, _e, _w, cx| this.apply(edit(up_v), cx)))
        .child("+");
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(56.))
        .px_4()
        .bg(rgb(theme::SURFACE))
        .rounded(px(10.))
        .child(div().text_color(rgb(theme::TEXT_MUTED)).child(label))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(down_btn)
                .child(
                    div()
                        .w(px(72.))
                        .text_color(rgb(theme::TEXT))
                        .child(format!("{value:.1}")),
                )
                .child(up_btn),
        );
    if in_flight {
        row = row.border_b_2().border_color(rgb(theme::ACCENT));
    }
    row
}

fn stepper_row_i32(
    label: &'static str,
    value: i32,
    step: i32,
    min: i32,
    max: i32,
    in_flight: bool,
    edit: impl Fn(i32) -> Edit + Copy + 'static,
    cx: &mut Context<CompositorPage>,
) -> gpui::Div {
    let down_v = (value - step).max(min);
    let up_v = (value + step).min(max);
    let down_btn = stepper_button_style()
        .id(SharedString::from(format!("dn:{label}")))
        .hover(|s| s.bg(rgb(theme::BG)))
        .on_click(cx.listener(move |this, _e, _w, cx| this.apply(edit(down_v), cx)))
        .child("−");
    let up_btn = stepper_button_style()
        .id(SharedString::from(format!("up:{label}")))
        .hover(|s| s.bg(rgb(theme::BG)))
        .on_click(cx.listener(move |this, _e, _w, cx| this.apply(edit(up_v), cx)))
        .child("+");
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(56.))
        .px_4()
        .bg(rgb(theme::SURFACE))
        .rounded(px(10.))
        .child(div().text_color(rgb(theme::TEXT_MUTED)).child(label))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(down_btn)
                .child(
                    div()
                        .w(px(72.))
                        .text_color(rgb(theme::TEXT))
                        .child(format!("{value}")),
                )
                .child(up_btn),
        );
    if in_flight {
        row = row.border_b_2().border_color(rgb(theme::ACCENT));
    }
    row
}

fn mode_picker_row(
    current: &str,
    in_flight: bool,
    cx: &mut Context<CompositorPage>,
) -> gpui::Div {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(56.))
        .px_4()
        .bg(rgb(theme::SURFACE))
        .rounded(px(10.))
        .child(div().text_color(rgb(theme::TEXT_MUTED)).child("Scaling mode"));
    let mut picker = div().flex().flex_row().gap_2();
    for &m in MODES {
        let is_cur = m == current;
        let mode_owned = m.to_string();
        let chip = div()
            .id(SharedString::from(format!("mode:{m}")))
            .px_3()
            .py_1()
            .rounded(px(6.))
            .cursor_pointer()
            .bg(rgb(if is_cur { theme::ACCENT } else { theme::SIDEBAR }))
            .text_color(rgb(if is_cur { theme::BG } else { theme::TEXT }))
            .on_click(cx.listener(move |this, _e, _w, cx| {
                this.apply(Edit::WallpaperMode(mode_owned.clone()), cx);
            }))
            .child(m.to_string());
        picker = picker.child(chip);
    }
    row = row.child(picker);
    if in_flight {
        row = row.border_b_2().border_color(rgb(theme::ACCENT));
    }
    row
}
