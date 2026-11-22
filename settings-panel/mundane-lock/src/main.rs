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
    width: i32,
    height: i32,
}
#[derive(Deserialize)]
struct Snapshot {
    version: u32,
    epoch: u64,
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
    Lost,
}
struct Locker {
    windows: HashMap<String, WindowHandle<Prompt>>,
    name: String,
    question: Option<Question>,
    revision: usize,
    status: String,
}
impl Global for Locker {}
struct Prompt {
    seen: usize,
    field: Option<Entity<TextField>>,
    submit: FocusHandle,
}
impl Prompt {
    fn answer(&mut self, cx: &mut Context<Self>) {
        if self.seen != cx.global::<Locker>().revision {
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
        if self.seen != state.revision {
            self.seen = state.revision;
            self.field = question.as_ref().map(|q| {
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
        let mut card = div()
            .w(rems(22.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(tokens::space::MD)
            .p(tokens::space::XL)
            .rounded(tokens::radius::DIALOG)
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.panel))
            .child(
                mundane_ui::text_label("title", "Session locked")
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(tokens::text::TITLE)
                    .child("Session locked"),
            )
            .child(mundane_ui::text_label("user", name.clone()).child(name));
        if !status.is_empty() {
            card = card.child(
                mundane_ui::Role::Status
                    .control("status", status.clone())
                    .a11y_synthetic_children(|tree| {
                        tree.parent_node().set_live(accesskit::Live::Polite);
                    })
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
            card = card.children(self.field.clone()).child(
                div().flex().justify_end().child(
                    mundane_ui::button("Unlock", &self.submit, t, click.clone())
                        .on_a11y_action(AccessibleAction::Click, move |_, w, cx| click(w, cx))
                        .bg(rgb(t.focus))
                        .text_color(rgb(t.surface)),
                ),
            );
        }
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(t.surface))
            .text_color(rgb(t.text))
            .text_size(tokens::text::BODY)
            .child(card)
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
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
        cx.spawn(async move |cx| {
            let mut epoch = None;
            while let Ok(event) = rx.recv().await {
                cx.update(|cx| match event {
                    Event::Outputs(s) => {
                        if epoch.is_some_and(|old| old != s.epoch)
                            || outputs(s.outputs, cx).is_err()
                        {
                            cx.quit();
                            return;
                        }
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
