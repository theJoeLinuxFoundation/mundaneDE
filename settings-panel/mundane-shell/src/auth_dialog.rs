use crate::{auth::Prompt, view::text_button, Service, Shell};
use gpui::{prelude::*, *};
use mundane_ui::{tokens, ChoiceMenu, MenuChoice, MenuEvent, TextField, Theme};
use zeroize::Zeroizing;

fn denial_offset(progress: f32) -> Pixels {
    px((progress * std::f32::consts::TAU * 2.).sin() * 5. * (1. - progress))
}

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
            .map(|(uid, name, full)| {
                MenuChoice::new(
                    uid.to_string(),
                    if full.is_empty() || full == name {
                        name.clone()
                    } else {
                        format!("{full} ({name})")
                    },
                )
            })
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
        let (_, name, full) = prompt
            .users
            .iter()
            .find(|u| u.0 == prompt.selected)
            .unwrap();
        let display = if full.is_empty() { name } else { full };
        let initials: String = display
            .split_whitespace()
            .take(2)
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect();
        let mut person = div()
            .flex()
            .items_center()
            .gap(tokens::space::MD)
            .child(
                div()
                    .size(rems(2.5))
                    .rounded_full()
                    .bg(rgb(theme.selected()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(initials),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        mundane_ui::text_label("full-name", display.clone())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(display.clone()),
                    )
                    .child(
                        mundane_ui::text_label("identity", format!("Administrator: {name}"))
                            .text_size(tokens::text::CAPTION)
                            .text_color(rgb(theme.muted()))
                            .child(format!("@{name}")),
                    ),
            );
        if prompt.users.len() > 1 {
            person = person.child(
                mundane_ui::menu_trigger("Change", &self.identities, theme, cx)
                    .border_0()
                    .text_color(rgb(theme.muted())),
            );
        }
        body = body.child(person).child(self.identities.clone());
        let view = cx.weak_entity();
        let disclosure = text_button("Details", &self.details, theme, move |_, cx| {
            let _ = view.update(cx, |v, cx| {
                v.expanded = !v.expanded;
                cx.notify()
            });
        })
        .aria_expanded(self.expanded)
        .border_0()
        .text_color(rgb(theme.muted()));
        body = body.child(div().flex().child(disclosure));
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
        if !prompt.status.is_empty() {
            body = body.child(
                mundane_ui::text_label("status", prompt.status.clone())
                    .text_color(rgb(theme.error()))
                    .child(prompt.status),
            )
        }
        if let Some((_, label, echo)) = prompt.challenge {
            if echo
                || !label
                    .trim()
                    .trim_end_matches(':')
                    .eq_ignore_ascii_case("password")
            {
                body = body.child(mundane_ui::text_label("question", label.clone()).child(label));
            }
            if let Some(f) = self.field.clone() {
                let field = div().relative().child(f);
                if prompt.failures > 0 && !cx.reduce_motion() {
                    body = body.child(field.with_animation(
                        ("denial", prompt.failures),
                        Animation::new(tokens::Motion::Transition.current(cx)),
                        |el, p| el.left(denial_offset(p)),
                    ));
                } else {
                    body = body.child(field);
                }
            }
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
            buttons = buttons.child(
                text_button("Authenticate", &self.submit, theme, move |_, cx| {
                    let _ = view.update(cx, |v, cx| v.answer(cx));
                })
                .bg(rgb(theme.focus))
                .text_color(rgb(theme.surface)),
            );
        }
        body.flex_none()
            .child(buttons)
            .on_children_prepainted(|bounds, w, cx| {
                if let Some(last) = bounds.last() {
                    let max = w
                        .display(cx)
                        .map(|d| d.bounds().size.height)
                        .unwrap_or(px(700.));
                    let h = (last.bottom() + w.rem_size() * 1.2).min(max - px(50.));
                    if (h - w.viewport_size().height).abs() > px(1.) {
                        w.resize(size(w.viewport_size().width, h));
                    }
                }
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denial_shake_decays_to_the_resting_field_position() {
        assert_eq!(denial_offset(0.), px(0.));
        assert!(denial_offset(0.125) > px(4.));
        assert!(denial_offset(0.375) < px(-3.));
        assert!(denial_offset(0.625) < denial_offset(0.125));
        assert_eq!(denial_offset(1.), px(0.));
    }
}
