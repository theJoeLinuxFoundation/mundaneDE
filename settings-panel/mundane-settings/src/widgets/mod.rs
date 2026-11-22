//! Bind the suite's controls to a Settings page and a stable model key.
pub mod prompt;
use gpui::{prelude::*, *};
use mundane_ui::{tokens, Theme};

pub fn focus(key: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(key, cx, |_, cx| cx.focus_handle().tab_stop(true))
        .read(cx)
        .clone()
}

pub fn action<T: 'static>(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<T>,
    activate: impl Fn(&mut T, &mut Context<T>) + 'static,
) -> Stateful<Div> {
    let view = cx.weak_entity();
    let focus = focus(key, window, cx);
    let invoke = move |_: &mut Window, cx: &mut App| {
        let _ = view.update(cx, |v, cx| activate(v, cx));
    };
    let invoke = std::rc::Rc::new(invoke);
    let accessible = invoke.clone();
    mundane_ui::button_base(label, &focus, Theme::current(cx), move |w, cx| {
        invoke(w, cx)
    })
    .on_a11y_action(AccessibleAction::Click, move |_, w, cx| accessible(w, cx))
    .px(tokens::space::MD)
    .py(tokens::space::SM)
}
