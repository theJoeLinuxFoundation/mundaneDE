//! Launcher layout and invocation of the suite's choice menu from a pointer or keyboard.
use crate::{catalog::Application, view::Surface, Event, Service, Shell};
use gpui::{prelude::*, *};
use mundane_ui::{tokens, ChoiceMenu, MenuChoice, MenuEvent, PanelState, Theme};
use std::{cell::Cell, collections::HashMap, rc::Rc, time::Duration};

pub(super) struct PinMenu {
    pub(super) choices: Entity<ChoiceMenu>,
    position: Point<Pixels>,
}

impl PinMenu {
    pub(super) fn overlay(&self, cx: &Context<Surface>) -> impl IntoElement {
        let menu = self.choices.clone();
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, {
                let menu = menu.clone();
                move |_, w, cx| menu.update(cx, |m, cx| m.dismiss(w, cx))
            })
            .child(
                anchored().position(self.position).snap_to_window().child(
                    div()
                        .occlude()
                        .w(rems(13.))
                        .bg(rgb(Theme::current(cx).panel))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(menu),
                ),
            )
    }
}

fn open_menu(
    view: &mut Surface,
    id: &str,
    trigger: &FocusHandle,
    tile: bool,
    pos: Point<Pixels>,
    w: &mut Window,
    cx: &mut Context<Surface>,
) {
    let s = cx.global::<Shell>();
    let Some(app) = s.apps.iter().find(|a| a.id == id) else {
        return;
    };
    let verb = if s.pins.contains(&app.id) {
        "Unpin"
    } else {
        "Pin"
    };
    let item = MenuChoice::new(id.to_owned(), format!("{verb} {}", app.name));
    let choices = cx.new(|cx| ChoiceMenu::new("Application actions", vec![item], false, cx));
    let trigger = trigger.clone();
    cx.subscribe_in(&choices, w, move |v, _, event, w, cx| {
        if let MenuEvent::Selected(id) = event {
            let _ = cx
                .global::<Shell>()
                .service
                .try_send(Service::Pin(id.to_string()));
        }
        v.pin_menu = None;
        if tile && matches!(event, MenuEvent::Selected(_)) {
            if let MenuEvent::Selected(id) = event {
                if let Some(f) = v.focus.get(&format!("app:{id}")) {
                    f.focus(w, cx);
                } else if let Some(f) = &v.field {
                    f.read(cx).focus_handle(cx).focus(w, cx);
                }
            }
        } else {
            trigger.focus(w, cx);
        }

        cx.notify();
    })
    .detach();
    choices.update(cx, |m, cx| m.show(w, cx));
    view.pin_menu = Some(PinMenu {
        choices,
        position: pos,
    });
    cx.notify();
}

fn app_control(
    app: &Application,
    tile: bool,
    pinned: bool,
    focus: &FocusHandle,
    window: &Window,
    scroll: &ScrollHandle,
    cx: &Context<Surface>,
) -> Stateful<Div> {
    let theme = Theme::current(cx);
    let icon = crate::view::contents(
        app.icon.clone(),
        &app.name,
        if tile {
            tokens::icon::EXTRA_LARGE
        } else {
            tokens::icon::LARGE
        },
    );
    let screen_position = Rc::new(Cell::new(Point::default()));
    let bounds_pos = screen_position.clone();
    let row_focus = focus.clone();
    let scroll = scroll.clone();
    let mut content = div()
        .on_children_prepainted(move |bounds, w, _| {
            if let Some(b) = bounds.first() {
                bounds_pos.set(b.bottom_left());
                crate::view::reveal_focus(*b, &row_focus, &scroll, w);
            }
        })
        .min_w_0()
        .flex()
        .items_center()
        .gap(tokens::space::SM)
        .child(icon);
    content = if tile {
        content
            .flex_col()
            .justify_center()
            .text_size(tokens::text::CAPTION)
    } else {
        content.flex_1()
    };
    content = content.child(div().truncate().child(app.name.clone()));
    if pinned && !tile {
        content = content
            .child(div().flex_1())
            .child(div().text_color(rgb(theme.muted())).child("★"));
    }
    let id = app.id.clone();
    let activate = {
        let id = id.clone();
        let view = cx.weak_entity();
        move |_: &mut Window, cx: &mut App| {
            let _ = view.update(cx, |v, cx| {
                if v.pin_menu.is_none() {
                    let _ = cx
                        .global::<Shell>()
                        .events
                        .try_send(Event::Service(Service::Launch(id.clone())));
                }
            });
        }
    };
    let menu = {
        let id = id.clone();
        let focus = focus.clone();
        move |v: &mut Surface, keyboard: bool, w: &mut Window, cx: &mut Context<Surface>| {
            let pos = if keyboard {
                screen_position.get()
            } else {
                w.mouse_position()
            };
            open_menu(v, &id, &focus, tile, pos, w, cx);
        }
    };
    let right = menu.clone();
    let key = menu.clone();
    let a11y = menu.clone();
    let view = cx.weak_entity();
    let button = if tile {
        mundane_ui::panel_button(
            app.name.clone(),
            content,
            focus,
            PanelState {
                active: focus.is_focused(window),
                ..Default::default()
            },
            theme,
            activate.clone(),
        )
        .w(rems(5.85))
        .px(tokens::space::XS)
        .py(tokens::space::SM)
        .on_a11y_action(AccessibleAction::Click, move |_, w, cx| activate(w, cx))
    } else {
        mundane_ui::launcher_row(
            app.name.clone(),
            focus,
            focus.is_focused(window),
            true,
            theme,
            activate,
        )
        .child(content)
    };
    button
        .aria_description(if pinned {
            "Pinned application"
        } else {
            "Application"
        })
        .on_a11y_action(AccessibleAction::ShowContextMenu, move |_, w, cx| {
            let _ = view.update(cx, |v, cx| a11y(v, true, w, cx));
        })
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |v, _, w, cx| right(v, false, w, cx)),
        )
        .on_key_down(cx.listener(move |v, e: &KeyDownEvent, w, cx| {
            if ["menu", "xf86menukb", "xf86contextmenu"]
                .iter()
                .any(|key| e.keystroke.key.eq_ignore_ascii_case(key))
                || (e.keystroke.key == "f10" && e.keystroke.modifiers.shift)
            {
                key(v, true, w, cx);
                cx.stop_propagation();
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |v, _, w, cx| {
                let menu = menu.clone();
                v.hold = Some(cx.spawn_in(w, async move |view, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(550))
                        .await;
                    let _ = view.update_in(cx, |v, w, cx| menu(v, false, w, cx));
                }));
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|v, _, _, _| {
                v.hold.take();
            }),
        )
        .on_hover(cx.listener(|v, over: &bool, _, _| {
            if !over {
                v.hold.take();
            }
        }))
}

pub(super) fn applications(
    s: &Shell,
    category: &str,
    query: &str,
    menu: &Entity<ChoiceMenu>,
    handles: &mut HashMap<String, FocusHandle>,
    window: &Window,
    scroll: &ScrollHandle,
    cx: &Context<Surface>,
) -> (Div, Option<Event>) {
    let mut body = div().flex().flex_col().gap(tokens::space::XS);
    let mut render = |a: &Application, tile| {
        let focus = handles
            .entry(format!("{}:{}", if tile { "tile" } else { "app" }, a.id))
            .or_insert_with(|| cx.focus_handle().tab_stop(true));
        app_control(a, tile, s.pins.contains(&a.id), focus, window, scroll, cx)
    };
    let all = category == "All applications";
    let search = !query.trim().is_empty();
    if !search && (all || category == "Favourites") {
        let mut grid = div().flex().flex_wrap().gap(tokens::space::XS);
        for a in s
            .pins
            .iter()
            .filter_map(|id| s.apps.iter().find(|a| &a.id == id))
        {
            grid = grid.child(render(a, true));
        }
        if !s.pins.is_empty() {
            body = body.child(section("Pinned")).child(grid);
        }
    }
    let mut apps: Vec<_> = s
        .apps
        .iter()
        .filter(|a| {
            if search {
                a.matches(query)
            } else if category == "Favourites" {
                s.pins.contains(&a.id)
            } else {
                all || a.categories.iter().any(|c| c == category)
            }
        })
        .collect();
    if search {
        let prefix = query.trim().to_lowercase();
        apps.sort_by_key(|a| !a.name.to_lowercase().starts_with(&prefix));
    }
    let first = apps
        .first()
        .map(|a| Event::Service(Service::Launch(a.id.clone())));
    if category != "Favourites" || search {
        body = body.child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(section(if search { "Search results" } else { category }))
                .when(!search, |el| {
                    el.child(mundane_ui::menu_trigger(
                        "Categories",
                        menu,
                        Theme::current(cx),
                        cx,
                    ))
                }),
        );

        for a in &apps {
            body = body.child(render(a, false));
        }
    }
    if !search && category == "Favourites" {
        body = body.child(mundane_ui::menu_trigger(
            "Categories",
            menu,
            Theme::current(cx),
            cx,
        ));
    }
    if apps.is_empty() {
        body = body.child("No matching applications");
    }
    (body, first)
}
fn section(name: &str) -> impl IntoElement {
    mundane_ui::text_label(
        SharedString::from(format!("heading-{name}")),
        name.to_owned(),
    )
    .px(tokens::space::SM)
    .pt(tokens::space::SM)
    .font_weight(FontWeight::SEMIBOLD)
    .child(name.to_owned())
}
