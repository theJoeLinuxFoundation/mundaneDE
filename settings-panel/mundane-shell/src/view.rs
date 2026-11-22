use crate::{Event, Service, Shell};
use gpui::{
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::*,
    *,
};
use mundane_ui::{tokens, ChoiceMenu, MenuChoice, MenuEvent, PanelState, TextField, Theme};
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Bar,
    Start,
    Workspaces,
    System,
    Group(String),
    Tray(String, Vec<i32>),
    Switcher,
}
impl Page {
    fn namespace(&self) -> &'static str {
        match self {
            Self::Bar => "mundane-taskbar",
            Self::Start => "mundane-start",
            Self::Workspaces => "mundane-overview",
            Self::System => "mundane-system",
            Self::Group(_) => "mundane-app-windows",
            Self::Tray(..) => "mundane-tray",
            Self::Switcher => "mundane-switcher",
        }
    }
    fn title(&self, s: &Shell) -> String {
        match self {
            Self::Bar => "Taskbar",
            Self::Start => "Start menu",
            Self::Workspaces => "Workspaces",
            Self::System => "System menu",
            Self::Group(id) => s
                .apps
                .iter()
                .find(|a| a.id == *id)
                .map(|a| a.name.as_str())
                .unwrap_or("Windows"),
            Self::Tray(k, _) => s
                .tray
                .iter()
                .find(|i| i.key == *k)
                .map(|i| i.title.as_str())
                .unwrap_or("Tray menu"),
            Self::Switcher => "Switch windows",
        }
        .to_string()
    }
}
pub struct Surface {
    page: Page,
    display: DisplayId,
    field: Option<Entity<TextField>>,
    focus: HashMap<String, FocusHandle>,
    category: String,
    choices: Option<Entity<ChoiceMenu>>,
    options: Vec<MenuChoice>,
    initial: bool,
}
pub fn open(
    display: DisplayId,
    page: Page,
    anchor_x: Option<f32>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Surface>> {
    let bounds = cx
        .displays()
        .into_iter()
        .find(|d| d.id() == display)
        .ok_or_else(|| anyhow::anyhow!("Output removed"))?
        .bounds();
    let bar = page == Page::Bar;
    let preview = page == Page::Switcher;
    let right = matches!(page, Page::Tray(..) | Page::System | Page::Workspaces);
    let width = if bar {
        f32::from(bounds.size.width)
    } else {
        (f32::from(bounds.size.width) - 16.).clamp(1., if preview { 360. } else { 410. })
    };
    let left = anchor_x.map(|x| {
        let x = if right { x - width + 20. } else { x - 20. };
        x.clamp(8., (f32::from(bounds.size.width) - width - 8.).max(8.))
    });
    let height = if bar {
        44.
    } else {
        (f32::from(bounds.size.height) - 60.).clamp(
            1.,
            match page {
                Page::System => (145. + 36. * cx.global::<Shell>().tray.len() as f32).min(340.),
                Page::Switcher => 85. + (cx.global::<Shell>().switcher.len() as f32) * 27.,
                Page::Group(_) | Page::Tray(..) => 232.,
                Page::Workspaces => {
                    (208. + 40. * cx.global::<Shell>().tasks.len() as f32).min(430.)
                }
                _ => 430.,
            },
        )
    };
    let title = page.title(cx.global::<Shell>());
    cx.open_window(
        WindowOptions {
            display_id: Some(display),
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(width), px(height)),
            ))),
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: page.namespace().into(),
                layer: if bar { Layer::Top } else { Layer::Overlay },
                keyboard_interactivity: if bar || preview {
                    KeyboardInteractivity::None
                } else {
                    KeyboardInteractivity::Exclusive
                },
                anchor: if bar {
                    Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT
                } else if preview {
                    Anchor::empty()
                } else {
                    Anchor::BOTTOM
                        | if right && left.is_none() {
                            Anchor::RIGHT
                        } else {
                            Anchor::LEFT
                        }
                },
                exclusive_zone: Some(px(if bar { 44. } else { 0. })),
                margin: if bar || preview {
                    None
                } else {
                    Some((px(0.), px(8.), px(6.), px(left.unwrap_or(8.))))
                },
                ..Default::default()
            }),
            ..Default::default()
        },
        move |w, cx| {
            w.set_window_title(&title);
            cx.new(|cx| {
                let theme = Theme::current(cx);
                let field = matches!(page, Page::Start | Page::Workspaces).then(|| {
                    cx.new(|cx| {
                        TextField::new(
                            if page == Page::Start {
                                "Search applications"
                            } else {
                                "Find a window"
                            },
                            theme,
                            cx,
                        )
                    })
                });
                if let Some(f) = &field {
                    cx.observe(f, |_, _, cx| cx.notify()).detach();
                }
                let s = cx.global::<Shell>();
                let has_pins = s.pins.iter().any(|id| s.apps.iter().any(|a| a.id == *id));
                let options = page.choices(s);
                let choices = matches!(
                    page,
                    Page::Start | Page::Tray(..) | Page::Group(_) | Page::System
                )
                .then(|| {
                    let menu = cx.new(|cx| {
                        ChoiceMenu::new(
                            if page == Page::Start {
                                "Application categories"
                            } else {
                                &title
                            },
                            options.clone(),
                            page == Page::Start,
                            cx,
                        )
                    });
                    cx.subscribe_in(&menu, w, |view: &mut Surface, _, evt, w, cx| {
                        let shell = cx.global::<Shell>();
                        match evt {
                            MenuEvent::Closed if view.page != Page::Start => {
                                let _ = shell.events.try_send(Event::Dismiss);
                            }
                            MenuEvent::Selected(k) if view.page == Page::Start => {
                                view.category = k.to_string();
                                if let Some(field) = &view.field {
                                    field.read(cx).focus_handle(cx).focus(w, cx);
                                }
                            }
                            MenuEvent::Selected(k) => {
                                if let Some(event) = view.page.selection(k, view.display, shell) {
                                    let _ = shell.events.try_send(event);
                                }
                            }
                            _ => {}
                        }
                        cx.notify();
                    })
                    .detach();
                    if page != Page::Start {
                        menu.update(cx, |m, cx| m.show(w, cx));
                    }
                    menu
                });
                Surface {
                    page,
                    display,
                    field,
                    category: if has_pins {
                        "Favourites"
                    } else {
                        "All applications"
                    }
                    .into(),
                    choices,
                    options,
                    focus: HashMap::new(),
                    initial: true,
                }
            })
        },
    )
}

// GPUI's default accessible Click synthesizes pointer input at the bounds centre.
// A scrolled-off launcher row still needs to activate its own action.
fn shell_button(
    label: String,
    focus: &FocusHandle,
    theme: Theme,
    activate: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    mundane_ui::button_base(label, focus, theme, activate.clone())
        .on_a11y_action(AccessibleAction::Click, move |_, w, cx| activate(w, cx))
}

struct Controls<'a> {
    first: Option<FocusHandle>,

    focus: &'a mut HashMap<String, FocusHandle>,
    theme: Theme,
    tx: async_channel::Sender<Event>,
}
impl Controls<'_> {
    fn button(&mut self, key: &str, name: &str, event: Event, cx: &App) -> Stateful<Div> {
        let focus = self
            .focus
            .entry(key.into())
            .or_insert_with(|| cx.focus_handle().tab_stop(true));
        self.first.get_or_insert_with(|| focus.clone());
        let tx = self.tx.clone();
        shell_button(name.to_owned(), focus, self.theme, move |_, _| {
            let _ = tx.try_send(event.clone());
        })
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .flex_shrink_0()
    }
    fn panel(
        &mut self,
        key: &str,
        name: &str,
        icon: impl IntoElement,
        event: Event,
        state: PanelState,
        cx: &App,
    ) -> Stateful<Div> {
        let focus = self
            .focus
            .entry(key.into())
            .or_insert_with(|| cx.focus_handle().tab_stop(true));
        let tx = self.tx.clone();
        let activate = move |w: &mut Window, _: &mut App| {
            let action = if let Event::Open(d, p) = &event {
                Event::OpenAt(*d, p.clone(), w.mouse_position().x.into())
            } else {
                event.clone()
            };
            let _ = tx.try_send(action);
        };
        let button = mundane_ui::panel_button(
            name.to_owned(),
            icon,
            focus,
            state,
            self.theme,
            activate.clone(),
        )
        .on_a11y_action(AccessibleAction::Click, move |_, w, cx| activate(w, cx));
        mundane_ui::with_tooltip(button, name.to_owned()).flex_shrink_0()
    }
}
fn contents(icon: Option<PathBuf>, name: &str) -> AnyElement {
    if let Some(path) = icon {
        img(path).size(tokens::icon::LARGE).into_any_element()
    } else {
        div().child(name.to_owned()).into_any_element()
    }
}
impl Render for Surface {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(menu) = &self.choices {
            let rows = self.page.choices(cx.global::<Shell>());
            let changed =
                self.options.len() != rows.len()
                    || self.options.iter().zip(&rows).any(|(a, b)| {
                        a.key != b.key || a.label != b.label || a.enabled != b.enabled
                    });
            if changed {
                let first = self.options.is_empty() && !rows.is_empty() && self.page != Page::Start;
                self.options = rows.clone();
                menu.update(cx, |m, cx| {
                    m.replace(rows, w, cx);
                    if first {
                        m.show(w, cx);
                    }
                });
            }
        }
        let s = cx.global::<Shell>();
        let theme = Theme::current(cx);
        let tx = s.events.clone();
        let display = self.display;
        let mut controls = Controls {
            first: None,
            focus: &mut self.focus,
            theme,
            tx: tx.clone(),
        };
        let mut root = div()
            .size_full()
            .flex()
            .bg(rgb(theme.panel))
            .text_color(rgb(theme.text))
            .font_family("DejaVu Sans")
            .text_size(tokens::text::CONTROL)
            .p(tokens::space::SM)
            .gap_2();
        let mut first = None;
        match &self.page {
            Page::Bar => {
                root = root
                    .flex_row()
                    .items_center()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .child(controls.panel(
                        "start",
                        "Start",
                        "Start",
                        Event::Open(display, Page::Start),
                        PanelState {
                            active: matches!(s.menu,Some((d,Page::Start,_)) if d==display),
                            ..Default::default()
                        },
                        cx,
                    ));
                let mut tasks = div()
                    .id("tasks")
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .flex()
                    .items_center()
                    .gap_1();
                for id in &s.pins {
                    let Some(a) = s.apps.iter().find(|a| a.id == *id) else {
                        continue;
                    };
                    let windows: Vec<_> = s.tasks.iter().filter(|t| a.owns(&t.app)).collect();
                    let event = match windows.as_slice() {
                        [] => Event::Service(Service::Launch(id.clone())),
                        [t] => Event::Command(format!("activate {}", t.id)),
                        _ => Event::Open(display, Page::Group(id.clone())),
                    };
                    let b = controls.panel(
                        id,
                        &a.name,
                        contents(a.icon.clone(), &a.name),
                        event,
                        PanelState {
                            running: !windows.is_empty(),
                            ..Default::default()
                        },
                        cx,
                    );
                    tasks = tasks.child(b);
                }
                for t in s.tasks.iter().filter(|t| {
                    t.workspace == s.desk
                        && !s
                            .apps
                            .iter()
                            .any(|a| s.pins.contains(&a.id) && a.owns(&t.app))
                }) {
                    let name = if t.title.is_empty() { &t.app } else { &t.title };
                    tasks = tasks.child(
                        controls
                            .panel(
                                &t.id,
                                name,
                                name.clone(),
                                Event::Command(format!("activate {}", t.id)),
                                PanelState {
                                    running: true,
                                    ..Default::default()
                                },
                                cx,
                            )
                            .max_w(px(175.))
                            .overflow_hidden(),
                    );
                }
                let ws = format!("Workspaces, current {}", s.desk);
                let mut markers = div().flex().gap(tokens::space::XS);
                for n in 1..=4 {
                    let count = s.tasks.iter().filter(|t| t.workspace == n).count();
                    let label = format!("Workspace {n}, {count} windows");
                    let marker = mundane_ui::badge(
                        ("desk", n as usize),
                        label.clone(),
                        n.to_string(),
                        theme,
                    )
                    .when(n == s.desk, |el| el.bg(rgb(theme.selected())))
                    .when(count > 0, |el| el.border_b_2());
                    markers = markers.child(mundane_ui::with_tooltip(marker, label));
                }
                let desks = controls.panel(
                    "workspaces",
                    &ws,
                    markers,
                    Event::Open(display, Page::Workspaces),
                    PanelState::default(),
                    cx,
                );
                let scroll = tx.clone();
                let desk = s.desk;
                root = root.child(tasks).child(desks.on_scroll(move |e, _, _| {
                    let d = e.delta.pixel_delta(px(1.));
                    let v = if d.y != px(0.) { d.y } else { d.x };
                    if v != px(0.) {
                        let n = if v < px(0.) {
                            desk % 4 + 1
                        } else {
                            (desk + 2) % 4 + 1
                        };
                        let _ = scroll.try_send(Event::Command(format!("workspace {n}")));
                    }
                }));
                for i in &s.tray {
                    let menu = Event::Open(display, Page::Tray(i.key.clone(), vec![0]));
                    let action = if i.is_menu && !i.menu.is_empty() {
                        menu.clone()
                    } else {
                        Event::Service(Service::TrayActivate(i.key.clone()))
                    };
                    let right = tx.clone();
                    let tray_page = Page::Tray(i.key.clone(), vec![0]);
                    root = root.child(
                        controls
                            .panel(
                                &i.key,
                                &i.title,
                                contents(i.icon.clone(), &i.title),
                                action,
                                PanelState::default(),
                                cx,
                            )
                            .when(!i.menu.is_empty(), |b| {
                                b.on_mouse_down(MouseButton::Right, move |_, w, _| {
                                    let _ = right.try_send(Event::OpenAt(
                                        display,
                                        tray_page.clone(),
                                        w.mouse_position().x.into(),
                                    ));
                                })
                            }),
                    );
                }
                root = root.child(controls.panel(
                    "clock",
                    "System menu",
                    s.clock.clone(),
                    Event::Open(display, Page::System),
                    PanelState::default(),
                    cx,
                ));
            }
            page => {
                let title = page.title(s);
                root = root
                    .flex_col()
                    .p_3()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .child(
                        mundane_ui::text_label("title", title.clone())
                            .text_size(tokens::text::TITLE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    );
                if let Some(f) = &self.field {
                    root = root.child(f.clone());
                }
                let query = self
                    .field
                    .as_ref()
                    .map(|f| f.read(cx).text().to_owned())
                    .unwrap_or_default();
                let mut list = div()
                    .id("choices")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1();
                match page {
                    Page::Start => {
                        let menu = self.choices.as_ref().unwrap();
                        let browsing = menu.read(cx).is_open();
                        if query.is_empty() {
                            root = root.child(
                                mundane_ui::menu_trigger("Browse categories", menu, theme, cx)
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(mundane_ui::badge(
                                        "current-category",
                                        "Current category",
                                        self.category.clone(),
                                        theme,
                                    ))
                                    .aria_description(self.category.clone()),
                            );
                        }
                        if browsing {
                            list = list.child(menu.clone());
                        } else {
                            let mut apps: Vec<_> = s
                                .apps
                                .iter()
                                .filter(|a| {
                                    if !query.trim().is_empty() {
                                        a.matches(&query)
                                    } else if self.category == "Favourites" {
                                        s.pins.contains(&a.id)
                                    } else {
                                        self.category == "All applications"
                                            || a.categories.contains(&self.category)
                                    }
                                })
                                .collect();
                            if !query.trim().is_empty() {
                                let prefix = query.trim().to_lowercase();
                                apps.sort_by_key(|a| !a.name.to_lowercase().starts_with(&prefix));
                            } else if self.category == "Favourites" {
                                apps.sort_by_key(|a| s.pins.iter().position(|id| id == &a.id));
                            }
                            if apps.is_empty() {
                                list = list.child("No matching applications");
                            }
                            for app in apps {
                                let action = Event::Service(Service::Launch(app.id.clone()));
                                first.get_or_insert(action.clone());
                                let name = &app.name;
                                let pinned = s.pins.contains(&app.id);
                                let pin =
                                    format!("{} {name}", if pinned { "Unpin" } else { "Pin" });
                                let mut launch =
                                    controls.button(&app.id, name, action, cx).flex_1();
                                if let Some(icon) = &app.icon {
                                    launch = launch.child(img(icon.clone()).size(px(22.)));
                                }
                                list = list.child(
                                    div()
                                        .flex()
                                        .gap_1()
                                        .child(launch.child(name.clone()))
                                        .child({
                                            let focus = controls
                                                .focus
                                                .entry(format!("pin:{}", app.id))
                                                .or_insert_with(|| {
                                                    cx.focus_handle().tab_stop(true)
                                                });
                                            let id = app.id.clone();
                                            let tx = tx.clone();
                                            let toggle =
                                                move |_: bool, _: &mut Window, _: &mut App| {
                                                    let _ = tx.try_send(Event::Service(
                                                        Service::Pin(id.clone()),
                                                    ));
                                                };
                                            mundane_ui::toggle(
                                                "Pin",
                                                focus,
                                                pinned,
                                                true,
                                                theme,
                                                toggle.clone(),
                                            )
                                            .flex()
                                            .flex_row()
                                            .gap(tokens::space::XS)
                                            .items_center()
                                            .whitespace_nowrap()
                                            .aria_label(pin)
                                            .on_a11y_action(
                                                AccessibleAction::Click,
                                                move |_, w, cx| toggle(!pinned, w, cx),
                                            )
                                        }),
                                );
                            }
                        }
                    }
                    Page::Workspaces => {
                        let mut desks = div().flex().gap(tokens::space::XS);
                        for n in 1..=4 {
                            let label = format!("Desk {n}");
                            let count = s.tasks.iter().filter(|t| t.workspace == n).count();
                            let status = if n == s.desk {
                                "Current".into()
                            } else if count == 0 {
                                "Empty".into()
                            } else {
                                format!("{count} open")
                            };
                            let content = div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .child(label.clone())
                                .child(div().text_size(tokens::text::CAPTION).child(status));
                            desks = desks.child(
                                controls
                                    .panel(
                                        &label,
                                        &label,
                                        content,
                                        Event::Command(format!("workspace {n}")),
                                        PanelState {
                                            active: n == s.desk,
                                            occupied: count > 0,
                                            ..Default::default()
                                        },
                                        cx,
                                    )
                                    .flex_1(),
                            );
                        }
                        root = root.child(desks);
                        for t in &s.tasks {
                            if !format!("{} {}", t.title, t.app)
                                .to_lowercase()
                                .contains(&query.to_lowercase())
                            {
                                continue;
                            }
                            let name = format!("{} · Desk {}", t.title, t.workspace);
                            let action = Event::Command(format!("activate {}", t.id));
                            first.get_or_insert(action.clone());
                            let mut row = div().flex().gap_1().child(
                                controls
                                    .button(&t.id, &name, action, cx)
                                    .flex_1()
                                    .child(name),
                            );
                            {
                                row = row
                                    .child(
                                        controls
                                            .button(
                                                &format!("move:{}", t.id),
                                                "Move →",
                                                Event::Command(format!(
                                                    "move {} {}",
                                                    t.id,
                                                    t.workspace % 4 + 1
                                                )),
                                                cx,
                                            )
                                            .child("Move →"),
                                    )
                                    .child(
                                        controls
                                            .button(
                                                &format!("close:{}", t.id),
                                                "Close window",
                                                Event::Command(format!("close {}", t.id)),
                                                cx,
                                            )
                                            .child("×"),
                                    );
                            }
                            list = list.child(row);
                        }
                    }
                    Page::Tray(..) | Page::Group(_) | Page::System => {
                        list = list.child(self.choices.as_ref().unwrap().clone())
                    }
                    Page::Switcher => {
                        for (i, title) in s.switcher.iter().enumerate() {
                            list = list.child(
                                mundane_ui::option(
                                    ("window", i).into(),
                                    title.clone(),
                                    i,
                                    i == s.selected,
                                    false,
                                )
                                .px(tokens::space::SM)
                                .when(i == s.selected, |el| el.bg(rgb(theme.selected())))
                                .child(title.clone()),
                            );
                        }
                        let caption =
                            format!("{} of {}  ·  Release Alt to switch", s.position, s.count);
                        list = list.child(
                            mundane_ui::text_label("selection", caption.clone()).child(caption),
                        );
                    }
                    Page::Bar => unreachable!(),
                }
                root = root.child(list);
                if *page == Page::Start {
                    root = root.child(
                        controls
                            .button(
                                "overview",
                                "Window overview",
                                Event::Open(display, Page::Workspaces),
                                cx,
                            )
                            .child("Window overview"),
                    );
                }
            }
        }
        let surface_title = self.page.title(s);
        if self.initial {
            if let Some(f) = &self.field {
                w.focus(&f.read(cx).focus_handle(cx), cx);
            } else if self.page != Page::Bar && self.page != Page::Switcher {
                if let Some(f) = controls.first.as_ref() {
                    w.focus(f, cx);
                }
            }
            self.initial = self.field.is_none() && controls.focus.is_empty();
        }
        let field = self.field.clone();
        let dismiss = tx.clone();
        let menu_page = self.page != Page::Bar && self.page != Page::Switcher;
        let root = root.on_key_down(move |ev, w, cx| {
            match ev.keystroke.key.as_str() {
                "escape" => {
                    let _ = tx.try_send(Event::Dismiss);
                }
                "down" => w.focus_next(cx),
                "up" => w.focus_prev(cx),
                "tab" => {
                    if ev.keystroke.modifiers.shift {
                        w.focus_prev(cx)
                    } else {
                        w.focus_next(cx)
                    }
                }
                "enter"
                    if field
                        .as_ref()
                        .is_some_and(|f| f.read(cx).focus_handle(cx).is_focused(w)) =>
                {
                    if let Some(action) = &first {
                        let _ = tx.try_send(action.clone());
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
        });
        if menu_page {
            mundane_ui::popover_contents(surface_title, theme, root, move |_, _| {
                let _ = dismiss.try_send(Event::Dismiss);
            })
            .p_0()
            .size_full()
            .into_any_element()
        } else {
            root.into_any_element()
        }
    }
}
