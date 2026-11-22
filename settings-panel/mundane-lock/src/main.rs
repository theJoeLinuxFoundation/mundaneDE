//! One GPUI process on a compositor-issued Wayland connection. Each ticket names one window.
mod pam;
use anyhow::{ensure, Context as _};
use async_channel::Sender;
use gpui::{prelude::*, *};
use mundane_ui::{tokens, ControlRole, TextField, Theme};
use nix::unistd::{Uid, User};
use serde::Deserialize;
use std::{
    collections::HashMap,
    os::{fd::AsFd, unix::net::UnixDatagram},
    thread,
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
struct Head {
    ticket: String,
    prompt: bool,
    width: i32,
    height: i32,
}
#[derive(Deserialize)]
struct Snapshot {
    version: u32,
    epoch: u64,
    awake: bool,
    outputs: Vec<Head>,
}
#[derive(Clone)]
struct Question {
    label: String,
    echo: bool,
    reply: Sender<Zeroizing<String>>,
}
enum Event {
    Outputs(Snapshot),
    Question(Question),
    Status(String),
    Clock(String, String),
    Lost,
}
struct Locker {
    windows: HashMap<String, WindowHandle<Prompt>>,
    active: Option<String>,
    awake: bool,
    time: String,
    date: String,
    name: String,
    question: Option<Question>,
    revision: usize,
    status: String,
}
impl Global for Locker {}
struct Prompt {
    ticket: String,
    visible: bool,
    fading: bool,
    transition: usize,
    seen: usize,
    field: Option<Entity<TextField>>,
    submit: FocusHandle,
}
impl Prompt {
    fn answer(&mut self, cx: &mut Context<Self>) {
        if self.seen != cx.global::<Locker>().revision
            || cx.global::<Locker>().active.as_ref() != Some(&self.ticket)
        {
            return;
        }
        let Some(field) = self.field.take() else {
            return;
        };
        let value = Zeroizing::new(field.read(cx).text().to_owned());
        let state = cx.global_mut::<Locker>();
        if let Some(question) = state.question.take() {
            state.revision += 1;
            state.status.clear();
            let _ = question.reply.try_send(value);
        }
        cx.refresh_windows();
    }
}
impl Render for Prompt {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::current(cx);
        let state = cx.global::<Locker>();
        let name = state.name.clone();
        let status = state.status.clone();
        let question = state.question.clone();
        let visible = state.active.as_ref() == Some(&self.ticket);
        let awake = state.awake;
        let time = state.time.clone();
        let date = state.date.clone();
        let revision = state.revision;
        if self.visible != visible {
            self.visible = visible;
            self.transition += 1;
            self.fading = !visible && !awake && !cx.reduce_motion();
            if self.fading {
                let transition = self.transition;
                let duration = tokens::Motion::Transition.current(cx);
                cx.spawn(async move |view, cx| {
                    cx.background_executor().timer(duration).await;
                    let _ = view.update(cx, |v, cx| {
                        if v.transition == transition {
                            v.fading = false;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            self.seen = usize::MAX;
        }
        if self.seen != revision {
            self.seen = revision;
            self.field = question.as_ref().filter(|_| visible).map(|q| {
                cx.new(|cx| {
                    if q.echo {
                        TextField::new("Response", t, cx)
                    } else {
                        TextField::password("Password", t, cx)
                    }
                })
            });
            if let Some(f) = &self.field {
                f.read(cx).focus_handle(cx).focus(w, cx);
            }
        }
        let white = rgb(0xffffff);
        let initials: String = name
            .split_whitespace()
            .take(2)
            .filter_map(|s| s.chars().next())
            .flat_map(char::to_uppercase)
            .collect();
        let mut card = div()
            .w(rems(17.))
            .max_w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(tokens::space::SM)
            .text_color(white)
            .child(
                div()
                    .size(rems(4.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(t.border))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(t.panel))
                    .text_color(rgb(t.text))
                    .text_size(rems(1.7))
                    .child(initials),
            )
            .child(
                mundane_ui::text_label("user", name.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(tokens::text::TITLE)
                    .child(name),
            );
        if !status.is_empty() {
            card = card.child(
                mundane_ui::Role::Status
                    .control("status", status.clone())
                    .a11y_synthetic_children(|tree| {
                        tree.parent_node().set_live(accesskit::Live::Polite);
                    })
                    .rounded(tokens::radius::CONTROL)
                    .px(tokens::space::SM)
                    .bg(rgb(t.panel))
                    .text_color(rgb(t.error()))
                    .child(status),
            );
        }
        if let Some(q) = question {
            if q.echo
                || !q
                    .label
                    .trim()
                    .trim_end_matches(':')
                    .eq_ignore_ascii_case("password")
            {
                card =
                    card.child(mundane_ui::text_label("challenge", q.label.clone()).child(q.label));
            }
            let view = cx.weak_entity();
            let click = move |_: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |v, cx| v.answer(cx));
            };
            let mut input = div()
                .w_full()
                .rounded(tokens::radius::CONTROL)
                .bg(rgb(t.surface))
                .text_color(rgb(t.text));
            if let Some(field) = self.field.clone() {
                input = input.child(field);
            } else {
                input = input.h(rems(2.));
            }
            card = card.child(input).child(
                mundane_ui::button("Unlock", &self.submit, t, click.clone())
                    .on_a11y_action(AccessibleAction::Click, move |_, w, cx| click(w, cx))
                    .w_full()
                    .flex()
                    .justify_center()
                    .bg(rgb(t.focus))
                    .text_color(rgb(t.surface)),
            );
        }
        let layout = format!("Keyboard · {}", cx.keyboard_layout().name());
        card = card.child(
            mundane_ui::text_label("layout", layout.clone())
                .mt(tokens::space::SM)
                .text_size(tokens::text::CAPTION)
                .child(layout),
        );
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .text_color(white)
            .text_size(tokens::text::BODY)
            .child(div().h(relative(0.21)))
            .child(
                mundane_ui::text_label("time", time.clone())
                    .text_size(rems(5.5))
                    .line_height(rems(6.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(time),
            );
        if awake || self.fading {
            root = root.child(
                mundane_ui::text_label("date", date.clone())
                    .text_size(rems(1.35))
                    .child(date),
            );
        }
        if visible || self.fading {
            root = root.child(div().h(relative(0.065)));
            let motion = !cx.reduce_motion() && self.transition > 0;
            root = if motion {
                root.child(card.with_animation(
                    ("prompt", self.transition),
                    Animation::new(tokens::Motion::Transition.current(cx)),
                    move |el, p| el.opacity(if visible { p } else { 1. - p }),
                ))
            } else {
                root.child(card)
            };
        }
        root.on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
            if e.keystroke.key == "enter"
                && v.field
                    .as_ref()
                    .is_some_and(|f| f.read(cx).focus_handle(cx).is_focused(w))
            {
                v.answer(cx);
                cx.stop_propagation();
            }
        }))
    }
}
fn outputs(heads: Vec<Head>, cx: &mut App) -> anyhow::Result<()> {
    let mut old = std::mem::take(&mut cx.global_mut::<Locker>().windows);
    for h in heads {
        let window = if let Some(w) = old.remove(&h.ticket) {
            w
        } else {
            cx.open_window(
                WindowOptions {
                    app_id: Some(h.ticket.clone()),
                    window_background: WindowBackgroundAppearance::Transparent,
                    titlebar: None,
                    window_decorations: Some(WindowDecorations::Server),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(h.width as f32), px(h.height as f32)),
                    ))),
                    ..Default::default()
                },
                |w, cx| {
                    w.set_window_title("Session locked");
                    cx.new(|cx| Prompt {
                        ticket: h.ticket.clone(),
                        visible: false,
                        fading: false,
                        transition: 0,
                        seen: usize::MAX,
                        field: None,
                        submit: cx.focus_handle().tab_stop(true),
                    })
                },
            )?
        };
        cx.global_mut::<Locker>().windows.insert(h.ticket, window);
    }
    for w in old.into_values() {
        let _ = w.update(cx, |_, w, _| w.remove_window());
    }
    Ok(())
}
fn main() -> anyhow::Result<()> {
    ensure!(
        std::env::var("WAYLAND_SOCKET").as_deref() == Ok("0"),
        "private locker connection required"
    );
    // Protect the inherited channels and PAM response buffers from same-user inspection.
    ensure!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } == 0,
        "protect locker process"
    );
    let socket = UnixDatagram::from(std::io::stdout().as_fd().try_clone_to_owned()?);
    socket.peer_addr()?;
    ensure!(unsafe { libc::dup2(2, 1) } == 1, "redirect stdout");
    let user = User::from_uid(Uid::current())?.context("unknown session user")?;
    let (tx, rx) = async_channel::unbounded();
    let input = socket.try_clone()?;
    let events = tx.clone();
    thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = input.recv(&mut buf) {
            let Ok(snapshot) = serde_json::from_slice::<Snapshot>(&buf[..n]) else {
                break;
            };
            if snapshot.version != 1 || events.send_blocking(Event::Outputs(snapshot)).is_err() {
                break;
            }
        }
        let _ = events.send_blocking(Event::Lost);
    });
    mundane_ui::run(move |cx| {
        let name = user
            .gecos
            .to_string_lossy()
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        cx.set_global(Locker {
            active: None,
            awake: true,
            time: String::new(),
            date: String::new(),
            windows: HashMap::new(),
            name: if name.is_empty() {
                user.name.clone()
            } else {
                name
            },
            question: None,
            revision: 0,
            status: String::new(),
        });
        cx.on_keyboard_layout_change(|cx| cx.refresh_windows())
            .detach();
        let clock = tx.clone();
        thread::spawn(move || {
            while let Ok(now) = glib::DateTime::now_local() {
                let event = Event::Clock(
                    now.format("%H:%M").unwrap_or_default().to_string(),
                    now.format("%A, %e %B").unwrap_or_default().to_string(),
                );
                if clock.send_blocking(event).is_err() {
                    break;
                }
                thread::sleep(std::time::Duration::from_secs(60 - now.second() as u64));
            }
        });
        cx.spawn(async move |cx| {
            let mut epoch = None;
            while let Ok(event) = rx.recv().await {
                cx.update(|cx| match event {
                    Event::Clock(time, date) => {
                        let state = cx.global_mut::<Locker>();
                        state.time = time;
                        state.date = date;
                        cx.refresh_windows();
                    }
                    Event::Outputs(s) => {
                        let state = cx.global_mut::<Locker>();
                        state.active = s
                            .outputs
                            .iter()
                            .find(|h| h.prompt)
                            .map(|h| h.ticket.clone());
                        state.awake = s.awake;
                        if epoch.is_some_and(|old| old != s.epoch)
                            || outputs(s.outputs, cx).is_err()
                        {
                            cx.quit();
                            return;
                        }
                        cx.refresh_windows();
                        if epoch.is_none() {
                            epoch = Some(s.epoch);
                            let Ok(channel) = socket.try_clone() else {
                                cx.quit();
                                return;
                            };
                            let tx = tx.clone();
                            let username = user.name.clone();
                            thread::spawn(move || pam::run(username, s.epoch, channel, tx));
                        }
                    }
                    Event::Question(q) => {
                        let state = cx.global_mut::<Locker>();
                        state.question = Some(q);
                        state.revision += 1;
                        cx.refresh_windows();
                    }
                    Event::Status(message) => {
                        cx.global_mut::<Locker>().status = message;
                        cx.refresh_windows();
                    }
                    Event::Lost => cx.quit(),
                });
            }
        })
        .detach();
    });
    Ok(())
}
