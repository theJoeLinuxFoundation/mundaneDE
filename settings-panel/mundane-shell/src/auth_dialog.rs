use crate::{auth::Prompt, view::text_button, Service, Shell};
use gpui::{prelude::*, *};
use mundane_ui::{tokens, ChoiceMenu, MenuChoice, MenuEvent, TextField, Theme};
use zeroize::Zeroizing;

pub struct Dialog {
    id: u64,
    identities: Entity<ChoiceMenu>,
    challenge: Option<u64>,
    field: Option<Entity<TextField>>,
    expanded: bool,
    details: FocusHandle,
    cancel: FocusHandle,
    submit: FocusHandle,
}
impl Dialog {
    pub fn new(prompt: &Prompt, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = prompt.id;
        let rows = prompt
            .users
            .iter()
            .map(|(uid, name)| MenuChoice::new(uid.to_string(), name.clone()))
            .collect();
        let identities = cx.new(|cx| ChoiceMenu::new("Administrator", rows, false, cx));
        cx.subscribe_in(&identities, w, move |_, _, event, _, cx| {
            if let MenuEvent::Selected(k) = event {
                if let Ok(uid) = k.parse() {
                    let _ = cx
                        .global::<Shell>()
                        .service
                        .try_send(Service::Identity(id, uid));
                }
            }
            cx.notify()
        })
        .detach();
        Self {
            id,
            identities,
            challenge: None,
            field: None,
            expanded: false,
            details: cx.focus_handle().tab_stop(true),
            cancel: cx.focus_handle().tab_stop(true),
            submit: cx.focus_handle().tab_stop(true),
        }
    }
    fn answer(&mut self, cx: &mut Context<Self>) {
        if let (Some(field), Some(seq)) = (self.field.take(), self.challenge) {
            let value = Zeroizing::new(field.read(cx).text().to_owned());
            let _ = cx
                .global::<Shell>()
                .service
                .try_send(Service::Answer(self.id, seq, value));
            cx.notify()
        }
    }
}
impl Render for Dialog {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let mut body = div().flex().flex_col().gap(tokens::space::SM);
        let Some(prompt) = cx
            .global::<Shell>()
            .auth
            .clone()
            .filter(|p| p.id == self.id)
        else {
            return body;
        };
        let seq = prompt.challenge.as_ref().map(|c| c.0);
        if self.challenge != seq {
            self.challenge = seq;
            self.field = prompt.challenge.as_ref().map(|(_, _, echo)| {
                cx.new(|cx| {
                    if *echo {
                        TextField::new("Response", theme, cx)
                    } else {
                        TextField::password("Password", theme, cx)
                    }
                })
            });
            if let Some(field) = &self.field {
                field.read(cx).focus_handle(cx).focus(w, cx)
            }
        }
        body = body
            .child(mundane_ui::text_label("message", prompt.message.clone()).child(prompt.message));
        let view = cx.weak_entity();
        body = body.child(
            text_button("Details", &self.details, theme, move |_, cx| {
                let _ = view.update(cx, |v, cx| {
                    v.expanded = !v.expanded;
                    cx.notify()
                });
            })
            .aria_expanded(self.expanded),
        );
        if self.expanded {
            body = body.child(
                mundane_ui::text_label("action-id", prompt.action.clone()).child(prompt.action),
            );
            for (key, value) in prompt.details {
                let line = format!("{key}: {value}");
                body = body.child(
                    mundane_ui::text_label(SharedString::from(key), line.clone()).child(line),
                );
            }
        }
        let name = prompt
            .users
            .iter()
            .find(|u| u.0 == prompt.selected)
            .map_or("", |u| u.1.as_str());
        let label = format!("Administrator: {name}");
        body = body.child(mundane_ui::text_label("identity", label.clone()).child(label));
        if prompt.users.len() > 1 {
            body = body
                .child(mundane_ui::menu_trigger(
                    "Choose administrator",
                    &self.identities,
                    theme,
                    cx,
                ))
                .child(self.identities.clone());
        }
        if !prompt.status.is_empty() {
            body = body
                .child(mundane_ui::text_label("status", prompt.status.clone()).child(prompt.status))
        }
        if let Some((_, label, _)) = prompt.challenge {
            body = body
                .child(mundane_ui::text_label("question", label.clone()).child(label))
                .children(self.field.clone());
        }
        let id = self.id;
        let cancel = cx.global::<Shell>().service.clone();
        let mut buttons = div()
            .flex()
            .justify_end()
            .gap(tokens::space::SM)
            .child(text_button("Cancel", &self.cancel, theme, move |_, _| {
                let _ = cancel.try_send(Service::CancelAuth(id));
            }));
        if self.field.is_some() {
            let view = cx.weak_entity();
            buttons = buttons.child(text_button(
                "Authenticate",
                &self.submit,
                theme,
                move |_, cx| {
                    let _ = view.update(cx, |v, cx| v.answer(cx));
                },
            ));
        }
        body.child(buttons)
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key == "enter"
                    && v.field
                        .as_ref()
                        .is_some_and(|f| f.read(cx).focus_handle(cx).is_focused(w))
                {
                    v.answer(cx);
                    cx.stop_propagation()
                }
            }))
    }
}
