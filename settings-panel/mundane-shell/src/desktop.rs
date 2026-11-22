//! Directory-backed icons on one bottom-layer surface per output.
use crate::{catalog, view, Event, Service, Shell};
use async_channel::Sender;
use gio::{glib, prelude::*};
use gpui::{
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::*,
    *,
};
use mundane_ui::{tokens, Theme};
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, PartialEq)]
pub struct Icon {
    uri: String,
    name: String,
    image: Option<PathBuf>,
}
fn entries(dir: &std::path::Path) -> Vec<Icon> {
    let theme = gio::Settings::new("org.gnome.desktop.interface").string("icon-theme");
    let mut icons = vec![
        Icon {
            name: "Home".into(),
            uri: gio::File::for_path(glib::home_dir()).uri().into(),
            image: catalog::icon_path("user-home", &theme),
        },
        Icon {
            name: "Trash".into(),
            uri: "trash:///".into(),
            image: catalog::icon_path("user-trash", &theme),
        },
    ];
    if let Ok(entries) = dir.read_dir() {
        for entry in entries.flatten() {
            let file = gio::File::for_path(entry.path());
            if let Ok(info) = file.query_info(
                "standard::display-name,standard::icon,standard::is-hidden",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            ) {
                if info.is_hidden() {
                    continue;
                }
                let image = info
                    .icon()
                    .and_then(|i| i.downcast::<gio::ThemedIcon>().ok())
                    .and_then(|i| {
                        i.names()
                            .iter()
                            .find_map(|name| catalog::icon_path(name, &theme))
                    });
                icons.push(Icon {
                    uri: file.uri().into(),
                    name: info.display_name().into(),
                    image,
                });
            }
        }
        icons[2..].sort_by_cached_key(|i| (i.name.to_lowercase(), i.uri.clone()));
    }
    icons
}
pub fn watch(tx: Sender<Event>) -> Option<gio::FileMonitor> {
    let dir = glib::user_special_dir(glib::UserDirectory::Desktop)
        .unwrap_or_else(|| glib::home_dir().join("Desktop"));
    let _ = std::fs::create_dir_all(&dir);

    let monitor = gio::File::for_path(&dir)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok();
    let refresh = move || {
        let _ = tx.try_send(Event::Icons(entries(&dir)));
    };
    refresh();
    if let Some(m) = &monitor {
        m.connect_changed(move |_, _, _, _| refresh());
    }
    monitor
}
pub struct Desktop {
    scroll: ScrollHandle,
    focus: HashMap<String, FocusHandle>,
}
impl Desktop {
    pub fn first(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = cx.global::<Shell>().icons.first() {
            if let Some(f) = self.focus.get(&i.uri) {
                self.scroll.scroll_to_item(0);
                f.focus(w, cx);
            }
        }
    }
}
impl Render for Desktop {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let s = cx.global::<Shell>();
        let factor = s.text_scale;
        let icons = s.icons.clone();
        let tx = s.service.clone();
        tokens::set_text_scale(w, factor);
        let size = w
            .display(cx)
            .map(|d| d.bounds().size)
            .unwrap_or(w.viewport_size());
        let tile_w = 96. * factor;
        let tile_h = 80. * factor;
        let rows = (((f32::from(size.height) - 52. * factor) / tile_h) as usize).max(1);
        let cols = icons.len().div_ceil(rows).max(1);
        let extent = gpui::size(
            px((cols as f32 * tile_w + 8.).min(f32::from(size.width))),
            px((rows.min(icons.len().max(1)) as f32 * tile_h + 8.).min(f32::from(size.height))),
        );
        if extent != w.viewport_size() {
            w.resize(extent);
        }
        self.focus
            .retain(|uri, _| icons.iter().any(|i| &i.uri == uri));
        let mut grid = div()
            .id("desktop")
            .size_full()
            .flex()
            .overflow_x_scroll()
            .track_scroll(&self.scroll)
            .p(px(4.));
        let mut handles = Vec::new();
        for col in icons.chunks(rows) {
            let mut column = div().flex().flex_col().flex_none();
            for icon in col {
                let focus = self
                    .focus
                    .entry(icon.uri.clone())
                    .or_insert_with(|| cx.focus_handle().tab_stop(true))
                    .clone();
                handles.push(focus.clone());
                let open = {
                    let tx = tx.clone();
                    let uri = icon.uri.clone();
                    move |_: &mut Window, _: &mut App| {
                        let _ = tx.try_send(Service::OpenUri(uri.clone()));
                    }
                };
                column = column.child(
                    mundane_ui::button_base(icon.name.clone(), &focus, theme, open.clone())
                        .on_a11y_action(AccessibleAction::Click, move |_, w, cx| open(w, cx))
                        .w(px(tile_w))
                        .h(px(tile_h))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .border_color(transparent_black())
                        .focus(move |s| s.bg(rgb(theme.selected())).border_color(rgb(theme.focus)))
                        .child(view::contents(
                            icon.image.clone(),
                            &icon.name,
                            tokens::icon::EXTRA_LARGE,
                        ))
                        .child(
                            div()
                                .max_w_full()
                                .truncate()
                                .px(tokens::space::XS)
                                .rounded(tokens::radius::CONTROL)
                                .bg(rgb(theme.panel))
                                .text_color(rgb(theme.text))
                                .text_size(tokens::text::CONTROL)
                                .child(icon.name.clone()),
                        ),
                );
            }
            grid = grid.child(column);
        }
        let scroll = self.scroll.clone();
        grid.on_key_down(move |event, w, cx| {
            let Some(i) = handles.iter().position(|f| f.is_focused(w)) else {
                return;
            };
            let max = handles.len() - 1;
            let next = match event.keystroke.key.as_str() {
                "up" => i.saturating_sub(1),
                "down" => (i + 1).min(max),
                "left" => i.saturating_sub(rows),
                "right" => (i + rows).min(max),
                "home" => 0,
                "end" => max,
                "enter" | "space" => {
                    let _ = tx.try_send(Service::OpenUri(icons[i].uri.clone()));
                    cx.stop_propagation();
                    return;
                }
                _ => return,
            };
            scroll.scroll_to_item(next / rows);
            handles[next].focus(w, cx);
            cx.stop_propagation();
        })
    }
}
pub fn open(display: DisplayId, cx: &mut App) -> anyhow::Result<WindowHandle<Desktop>> {
    cx.open_window(
        WindowOptions {
            display_id: Some(display),
            titlebar: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(104.), px(168.)),
            ))),
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: "mundane-desktop-icons".into(),
                layer: Layer::Bottom,
                anchor: Anchor::TOP | Anchor::LEFT,
                keyboard_interactivity: KeyboardInteractivity::OnDemand,
                exclusive_zone: Some(px(0.)),
                ..Default::default()
            }),
            ..Default::default()
        },
        |w, cx| {
            w.set_window_title("Desktop icons");
            cx.new(|_| Desktop {
                focus: HashMap::new(),
                scroll: ScrollHandle::new(),
            })
        },
    )
}
