//! GPUI forms resolve on Submit, Escape, Cancel or window close.
use anyhow::Result;
use gpui::{prelude::*, *};
use mundane_ui::{tokens, TextField, Theme};

struct Form {
    title: &'static str,
    detail: String,
    fields: Vec<Entity<TextField>>,
    cancel: FocusHandle,
    submit: FocusHandle,
    result: async_channel::Sender<Vec<String>>,
}
impl Form {
    fn submit(&self, w: &mut Window, cx: &App) {
        let values: Vec<_> = self
            .fields
            .iter()
            .map(|f| f.read(cx).text().to_owned())
            .collect();
        if values.iter().all(|v| !v.is_empty()) {
            let _ = self.result.try_send(values);
            w.remove_window();
        }
    }
}
impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let mut form = div()
            .size_full()
            .p(tokens::space::LG)
            .gap(tokens::space::MD)
            .flex()
            .flex_col()
            .bg(rgb(theme.surface))
            .text_color(rgb(theme.text))
            .child(mundane_ui::heading("form-heading", self.title));
        if !self.detail.is_empty() {
            form = form.child(self.detail.clone());
        }
        for f in &self.fields {
            form = form.child(f.clone());
        }
        let view = cx.weak_entity();
        let actions = div()
            .flex()
            .justify_end()
            .gap(tokens::space::SM)
            .child(mundane_ui::button("Cancel", &self.cancel, theme, |w, _| {
                w.remove_window()
            }))
            .child(mundane_ui::button(
                "Submit",
                &self.submit,
                theme,
                move |w, cx| {
                    let _ = view.update(cx, |v, cx| v.submit(w, cx));
                },
            ));
        form.child(actions)
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                match e.keystroke.key.as_str() {
                    "escape" => w.remove_window(),
                    "tab" if e.keystroke.modifiers.shift => w.focus_prev(cx),
                    "tab" => w.focus_next(cx),
                    "enter"
                        if v.fields
                            .iter()
                            .any(|f| f.read(cx).focus_handle(cx).is_focused(w)) =>
                    {
                        v.submit(w, cx)
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
    }
}
async fn form(
    title: &'static str,
    detail: String,
    labels: &[&'static str],
    masked: bool,
    cx: &mut AsyncApp,
) -> Result<Option<Vec<String>>> {
    let (tx, rx) = async_channel::bounded(1);
    let bounds = cx.update(|cx| Bounds::centered(None, size(px(440.), px(250.)), cx));
    cx.open_window(
        WindowOptions {
            kind: WindowKind::Dialog,
            app_id: Some("org.joelinux.MundaneSettings".into()),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        |w, cx| {
            w.set_window_title(title);
            cx.new(|cx| {
                let fields: Vec<_> = labels
                    .iter()
                    .map(|label| {
                        cx.new(|cx| {
                            if masked {
                                TextField::password(label, Theme::current(cx), cx)
                            } else {
                                TextField::new(label, Theme::current(cx), cx)
                            }
                        })
                    })
                    .collect();
                w.focus(&fields[0].read(cx).focus_handle(cx), cx);
                Form {
                    title,
                    detail,
                    fields,
                    cancel: cx.focus_handle().tab_stop(true),
                    submit: cx.focus_handle().tab_stop(true),
                    result: tx,
                }
            })
        },
    )?;
    Ok(rx.recv().await.ok())
}
pub async fn password(
    title: &'static str,
    label: &str,
    cx: &mut AsyncApp,
) -> Result<Option<String>> {
    Ok(form(title, label.into(), &["Password"], true, cx)
        .await?
        .and_then(|mut vs| vs.pop()))
}
pub async fn name_and_url(
    title: &'static str,
    name: &'static str,
    url: &'static str,
    cx: &mut AsyncApp,
) -> Result<Option<(String, String)>> {
    Ok(form(title, String::new(), &[name, url], false, cx)
        .await?
        .and_then(|v| {
            let (name, url) = (v[0].trim(), v[1].trim());
            (!name.is_empty() && !url.is_empty()).then(|| (name.into(), url.into()))
        }))
}
