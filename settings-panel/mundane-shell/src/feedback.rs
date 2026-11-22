//! Passive layer surfaces do not take keyboard focus or replace the active menu.
use crate::{notifications::Note, Service, Shell};
use gpui::{
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::*,
    *,
};
use mundane_ui::{tokens, NoticeAction, NotificationCard, Theme};
use std::time::Duration;
#[derive(Clone)]
pub enum Feedback {
    Banner(Note),
    Level(&'static str, u8, bool),
}
pub struct Passive {
    value: Feedback,
    card: Option<NotificationCard>,
}
pub fn action(id: SharedString, action: NoticeAction, _: &mut Window, cx: &mut App) {
    if let Ok(id) = id.parse() {
        let key = match action {
            NoticeAction::Dismiss => None,
            NoticeAction::Invoke(k) => Some(k.to_string()),
        };
        let _ = cx
            .global::<Shell>()
            .service
            .try_send(Service::NoteAction(id, key));
    }
}
impl Render for Passive {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tokens::set_text_scale(w, cx.global::<Shell>().text_scale);
        let t = Theme::current(cx);
        let content = match &self.value {
            Feedback::Banner(note) => {
                let card = self
                    .card
                    .get_or_insert_with(|| NotificationCard::new(note.card(), cx));
                card.update(note.card(), cx);
                card.render(t, action)
            }
            Feedback::Level(label, value, muted) => {
                mundane_ui::level_osd("level", *label, *value, *muted, t)
            }
        };
        content
            .size_full()
            .text_color(rgb(t.text))
            .text_size(tokens::text::CONTROL)
    }
}
#[derive(Default)]
pub struct Overlays {
    banner: Option<(u32, WindowHandle<Passive>)>,
    osd: Option<WindowHandle<Passive>>,
    banner_timer: Option<Task<()>>,
    osd_timer: Option<Task<()>>,
}
pub fn hide_banner(id: Option<u32>, cx: &mut App) {
    let f = &mut cx.global_mut::<Shell>().overlays;
    if f.banner.is_some_and(|(n, _)| id.is_none_or(|id| n == id)) {
        f.banner_timer.take();
        if let Some((_, w)) = f.banner.take() {
            let _ = w.update(cx, |_, w, _| w.remove_window());
        }
    }
}
pub fn hide_level(cx: &mut App) {
    let f = &mut cx.global_mut::<Shell>().overlays;
    f.osd_timer.take();
    if let Some(w) = f.osd.take() {
        let _ = w.update(cx, |_, w, _| w.remove_window());
    }
}
pub fn show(value: Feedback, timeout: i32, cx: &mut App) {
    let s = cx.global::<Shell>();
    if s.locked {
        return;
    }
    let Some(d) = crate::home_display(cx) else {
        return;
    };
    let scale = s.text_scale;
    let banner = matches!(&value, Feedback::Banner(_));
    let (title, width, height) = match &value {
        Feedback::Banner(n) => (
            "Notification",
            395.,
            (92. + 22.
                * ((n.body.chars().count() / 44 + n.title.chars().count() / 34).min(5) as f32)
                + 30. * ((n.actions.len() / 3 + 1).min(3) as f32)),
        ),
        Feedback::Level(label, ..) => (*label, 275., 65.),
    };
    let size = size(
        px((width * scale).min(d.bounds().size.width.into())),
        px((height * scale).min(d.bounds().size.height.into())),
    );
    let old = if banner {
        s.overlays.banner.map(|(_, w)| w)
    } else {
        s.overlays.osd
    };
    let id = match &value {
        Feedback::Banner(n) => n.id,
        _ => 0,
    };
    let win = if let Some(w) = old.filter(|w| {
        w.update(cx, |v, w, cx| {
            v.value = value.clone();
            w.resize(size);
            w.set_window_title(title);
            cx.notify()
        })
        .is_ok()
    }) {
        w
    } else {
        let Ok(w) = cx.open_window(
            WindowOptions {
                display_id: Some(d.id()),
                titlebar: None,
                focus: false,
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size,
                ))),
                kind: WindowKind::LayerShell(LayerShellOptions {
                    layer: Layer::Overlay,
                    namespace: if banner {
                        "mundane-notification"
                    } else {
                        "mundane-osd"
                    }
                    .into(),
                    anchor: if banner {
                        Anchor::TOP | Anchor::RIGHT
                    } else {
                        Anchor::BOTTOM
                    },
                    margin: Some((px(12.), px(12.), px(18.), px(0.))),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    exclusive_zone: Some(px(0.)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |w, cx| {
                w.set_window_title(title);
                cx.new(|_| Passive { value, card: None })
            },
        ) else {
            return;
        };
        w
    };
    let timer = (timeout != 0).then(|| {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(if timeout < 0 {
                    5000
                } else {
                    timeout as u64
                }))
                .await;
            cx.update(|cx| {
                if banner {
                    hide_banner(Some(id), cx)
                } else {
                    hide_level(cx)
                }
            });
        })
    });
    let f = &mut cx.global_mut::<Shell>().overlays;
    if banner {
        f.banner = Some((id, win));
        f.banner_timer = timer;
    } else {
        f.osd = Some(win);
        f.osd_timer = timer;
    }
}
