//! Compare keyboard and application edits in the same AccessKit text field.
use gpui::{prelude::*, *};
use mundane_ui::{TextField, Theme};
use std::{path::PathBuf, time::Duration};

struct Example(Entity<TextField>);
impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.clone())
    }
}
fn main() {
    gpui_platform::application().run(|cx| {
        mundane_ui::bind_text_keys(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
            window.set_window_title("Echo test");
            let field = cx.new(|cx| TextField::new("City", Theme::DARK, cx));
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "Tokyo", window, cx)
            });
            field.read(cx).focus_handle(cx).focus(window, cx);
            cx.new(|cx| {
                let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap());
                cx.spawn_in(window, async move |view, cx| loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    if view
                        .update_in(cx, |view: &mut Example, window, cx| {
                            view.0.update(cx, |field, cx| {
                                if std::fs::remove_file(runtime.join("append-character")).is_ok() {
                                    field.replace_text_in_range(Some(5..5), "z", window, cx);
                                }
                                if field.text() == "Tokyoz" {
                                    std::fs::write(runtime.join("echo-value"), field.text())
                                        .unwrap();
                                }
                            });
                        })
                        .is_err()
                    {
                        break;
                    }
                })
                .detach();
                Example(field)
            })
        })
        .unwrap();
        cx.activate(true);
    });
}
