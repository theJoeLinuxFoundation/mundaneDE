//! Compositor page — the page that exists for mundaneDE specifically.
//!
//! Every control writes through to `~/.config/mundane/config.toml` via
//! `mundane_settings_core::compositor::update`. The mundaneDE compositor
//! watches that file and applies the diff live — change a slider here, see the
//! result in the next frame.

use crate::pages::{Page, PageState};
use crate::theme;
use gpui::{
    div, px, rgb, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window,
};
use mundane_settings_core::compositor as backend;

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
    saving: bool,
    pending: Vec<Edit>,
    last_error: Option<String>,
    sliders: [mundane_ui::ValueSlider; 6],
    modes: [gpui::FocusHandle; 5],
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
        let s = Self {
            state: PageState::Empty,
            sliders: std::array::from_fn(|_| mundane_ui::ValueSlider::new(cx)),
            modes: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            saving: false,
            pending: Vec::new(),
            last_error: None,
        };
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
        let PageState::Loaded { data, .. } = &mut self.state else {
            return;
        };
        match &edit {
            Edit::Stiffness(v) => data.stiffness = *v,
            Edit::Damping(v) => data.damping = *v,
            Edit::Mass(v) => data.mass = *v,
            Edit::Radius(v) => data.attraction_radius = *v,
            Edit::OuterGap(v) => data.outer_gap = *v,
            Edit::InnerGap(v) => data.inner_gap = *v,
            Edit::WallpaperMode(m) => data.wallpaper_mode = m.clone(),
        }
        self.pending
            .retain(|old| std::mem::discriminant(old) != std::mem::discriminant(&edit));
        self.pending.push(edit);
        self.last_error = None;
        cx.notify();
        if !self.saving {
            self.save(cx);
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.saving = true;
        let batch = std::mem::take(&mut self.pending);
        cx.spawn(async move |weak, cx| {
            let result = backend::update(move |f| {
                for edit in batch {
                    let (k, d, m) = f.spring();
                    let (radius, outer, inner) = f.wm();
                    match edit {
                        Edit::Stiffness(v) => f.set_spring(v, d, m),
                        Edit::Damping(v) => f.set_spring(k, v, m),
                        Edit::Mass(v) => f.set_spring(k, d, v),
                        Edit::Radius(v) => f.set_wm(v, outer, inner),
                        Edit::OuterGap(v) => f.set_wm(radius, v, inner),
                        Edit::InnerGap(v) => f.set_wm(radius, outer, v),
                        Edit::WallpaperMode(mode) => f.set_wallpaper_mode(&mode),
                    }
                }
            })
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.saving = false;
                if let Err(error) = result {
                    this.pending.clear();
                    this.last_error = Some(error.to_string());
                    this.spawn_refresh(cx);
                } else if !this.pending.is_empty() {
                    this.save(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// Damping ratio ζ = damping / (2 * sqrt(stiffness * mass)).
fn zeta(stiffness: f64, damping: f64, mass: f64) -> f64 {
    let denom = 2.0 * (stiffness * mass).sqrt();
    if denom <= 0.0 {
        0.0
    } else {
        damping / denom
    }
}

impl Render for CompositorPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.saving && self.state.should_refresh(Page::Compositor.ttl()) {
            self.spawn_refresh(cx);
        }

        let last_error = self.last_error.clone();
        let body = match &self.state {
            PageState::Loaded { data, .. } => self.data_view(data.clone(), cx),
            _ => skeleton(),
        };
        let mut root = div()
            .id("compositor-scroll")
            .overflow_y_scroll()
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
                    .text_color(rgb(theme::palette().secondary))
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
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .child(
            div()
                .text_color(rgb(theme::palette().secondary))
                .child(label),
        )
        .child(
            div()
                .w(px(180.))
                .h(px(28.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(6.)),
        )
}

impl CompositorPage {
    fn data_view(&self, d: CompositorData, cx: &Context<Self>) -> gpui::Div {
        let mut col = div().flex().flex_col().gap_2();
        let theme = mundane_ui::Theme::current(cx);
        for (idx, (label, value, min, max, step)) in [
            ("Stiffness", d.stiffness, 1., 500., 1.),
            ("Damping", d.damping, 0., 60., 1.),
            ("Mass", d.mass, 0.1, 10., 0.1),
            (
                "Attraction radius (px)",
                d.attraction_radius.into(),
                0.,
                96.,
                1.,
            ),
            ("Outer gap (px)", d.outer_gap.into(), 0., 32., 1.),
            ("Inner gap (px)", d.inner_gap.into(), 0., 32., 1.),
        ]
        .into_iter()
        .enumerate()
        {
            if idx == 0 {
                col = col.child(mundane_ui::heading("springs", "Spring physics"));
            }
            if idx == 3 {
                col = col
                    .child(format!(
                        "ζ (damping ratio) = {:.2}",
                        zeta(d.stiffness, d.damping, d.mass)
                    ))
                    .child(mundane_ui::heading("magnetic", "Magnetic window manager"));
            }
            let view = cx.weak_entity();
            col = col.child(self.sliders[idx].render(
                label,
                mundane_ui::SliderRange::new(min, max, step).unwrap(),
                value,
                true,
                theme,
                move |v, _, cx| {
                    let edit = match idx {
                        0 => Edit::Stiffness(v),
                        1 => Edit::Damping(v),
                        2 => Edit::Mass(v),
                        3 => Edit::Radius(v as i32),
                        4 => Edit::OuterGap(v as i32),
                        _ => Edit::InnerGap(v as i32),
                    };
                    let _ = view.update(cx, |page, cx| page.apply(edit, cx));
                },
            ));
        }
        let mut modes = div().flex().gap_2();
        for (&mode, focus) in MODES.iter().zip(&self.modes) {
            let view = cx.weak_entity();
            modes = modes.child(
                mundane_ui::toggle(
                    mode,
                    focus,
                    d.wallpaper_mode == mode,
                    true,
                    theme,
                    move |_, _, cx| {
                        let _ = view.update(cx, |page, cx| {
                            page.apply(Edit::WallpaperMode(mode.into()), cx)
                        });
                    },
                )
                .aria_label(format!("Wallpaper mode {mode}")),
            );
        }
        col.child(mundane_ui::heading("wallpaper", "Wallpaper scaling mode"))
            .child(modes)
    }
}
