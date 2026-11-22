//! Appearance page. Click a value row to expand a list of options below it —
//! selecting an item updates immediately. The single-open pattern keeps the
//! layout tight; clicking the same row again collapses.
//!
//! GTK theme and icon theme write through gsettings (they affect every GTK app
//! on the system regardless of compositor). The wallpaper row writes through
//! to mundaneDE's TOML config so the compositor's hot-reload picks it up and
//! crossfades.

use crate::pages::{Page, PageState};
use crate::theme;
use gpui::{
    div, px, rgb, AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use mundane_settings_core::{compositor as mundane_cfg, theme_scan};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct AppearanceData {
    pub gtk_theme: String,
    pub icon_theme: String,
    pub interface_font: String,
    pub monospace_font: String,
    pub wallpaper_uri: String,
    pub families: Vec<String>,
    pub themes_available: Vec<String>,
    pub icons_available: Vec<String>,
    pub wallpapers_available: Vec<PathBuf>,
}

/// Which section's option list is expanded. At most one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    GtkTheme,
    IconTheme,
    InterfaceFont,
    MonospaceFont,
    Wallpaper,
}

pub struct AppearancePage {
    state: PageState<AppearanceData>,
    in_flight: HashSet<&'static str>,
    open: Option<Section>,
    controls: [gpui::FocusHandle; 5],
    error: Option<String>,
}

impl AppearancePage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = Self {
            controls: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            error: None,
            state: PageState::Empty,
            in_flight: Default::default(),
            open: None,
        };
        s.spawn_refresh(cx);
        s
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, async_cx| {
            let themes = theme_scan::list_gtk_themes().await.unwrap_or_default();
            let icons = theme_scan::list_icon_themes().await.unwrap_or_default();
            let wallpapers = mundane_settings_core::wallpaper_scan::list_wallpapers()
                .await
                .unwrap_or_default();
            let gtk_theme = theme_scan::gsettings_get("org.gnome.desktop.interface", "gtk-theme")
                .await
                .unwrap_or_default();
            let icon_theme = theme_scan::gsettings_get("org.gnome.desktop.interface", "icon-theme")
                .await
                .unwrap_or_default();
            let interface_font =
                theme_scan::gsettings_get("org.gnome.desktop.interface", "font-name")
                    .await
                    .unwrap_or_default();
            let monospace_font =
                theme_scan::gsettings_get("org.gnome.desktop.interface", "monospace-font-name")
                    .await
                    .unwrap_or_default();
            // mundaneDE wallpaper lives in ~/.config/mundane/config.toml as a plain path;
            // we show it in the same "file://…" form used for the equality check below.
            let wallpaper_uri = match mundane_cfg::load().await {
                Ok(file) => file
                    .wallpaper_path()
                    .map(|p| format!("file://{p}"))
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };

            let data = AppearanceData {
                families: theme_scan::font_families().await.unwrap_or_default(),
                gtk_theme,
                icon_theme,
                interface_font,
                monospace_font,
                wallpaper_uri,
                themes_available: themes,
                icons_available: icons,
                wallpapers_available: wallpapers,
            };
            weak.update(async_cx, |this, cx| {
                this.state = PageState::Loaded {
                    data,
                    fetched_at: std::time::Instant::now(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.open = if self.open == Some(section) {
            None
        } else {
            Some(section)
        };
        cx.notify();
    }

    fn pick_gtk_theme(&mut self, name: String, cx: &mut Context<Self>) {
        self.set_gsettings(
            "gtk_theme",
            "org.gnome.desktop.interface",
            "gtk-theme",
            name,
            |d, v| d.gtk_theme = v,
            cx,
        );
    }

    fn pick_icon_theme(&mut self, name: String, cx: &mut Context<Self>) {
        self.set_gsettings(
            "icon_theme",
            "org.gnome.desktop.interface",
            "icon-theme",
            name,
            |d, v| d.icon_theme = v,
            cx,
        );
    }

    fn pick_wallpaper(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Optimistic local update first.
        let uri = format!("file://{}", path.display());
        if let PageState::Loaded { data, .. } = &mut self.state {
            data.wallpaper_uri = uri;
        }
        self.in_flight.insert("wallpaper");
        self.open = None;
        cx.notify();

        let path_str = path.to_string_lossy().into_owned();
        cx.spawn(async move |weak, async_cx| {
            let _ = mundane_cfg::update(move |f| {
                let mode = f.wallpaper_mode();
                f.set_wallpaper(&path_str, &mode);
            })
            .await;
            weak.update(async_cx, |this, cx| {
                this.in_flight.remove("wallpaper");
                cx.notify();
                this.spawn_refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn set_gsettings(
        &mut self,
        key: &'static str,
        schema: &'static str,
        gkey: &'static str,
        value: String,
        apply_local: impl FnOnce(&mut AppearanceData, String) + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        // Optimistic update.
        if let PageState::Loaded { data, .. } = &mut self.state {
            apply_local(data, value.clone());
        }
        self.in_flight.insert(key);
        self.open = None;
        cx.notify();
        cx.spawn(async move |weak, async_cx| {
            let error = theme_scan::gsettings_set(schema, gkey, &value)
                .await
                .err()
                .map(|e| e.to_string());
            weak.update(async_cx, |this, cx| {
                this.error = error;
                this.in_flight.remove(key);
                cx.notify();
                this.spawn_refresh(cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for AppearancePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.should_refresh(Page::Appearance.ttl()) {
            self.spawn_refresh(cx);
        }
        let body = match &self.state {
            PageState::Loaded { data, .. } => data_view(data, &self.in_flight, self.open, cx),
            _ => skeleton(),
        };
        use mundane_ui::ControlRole;
        let prefs = theme::current();
        let palette = mundane_ui::Theme::current(cx);
        let dark = !palette.light;
        let contrast = palette.high_contrast;
        let reduced = cx.reduce_motion();
        let mut form = div()
            .id("appearance-scroll")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .gap_2()
            .child(mundane_ui::heading("appearance-heading", "Appearance"));
        let choices = [
            "blue", "teal", "green", "yellow", "orange", "red", "pink", "purple", "slate",
        ];
        let accent = choices
            [(choices.iter().position(|s| *s == prefs.accent).unwrap_or(0) + 1) % choices.len()];
        for (idx, (id, label, value, schema, key, next, toggle)) in [
            (
                "dark",
                "Dark appearance",
                dark.to_string(),
                "org.gnome.desktop.interface",
                "color-scheme",
                if dark {
                    "prefer-light"
                } else {
                    "prefer-dark"
                },
                Some(dark),
            ),
            (
                "contrast",
                "High contrast",
                contrast.to_string(),
                "org.gnome.desktop.a11y.interface",
                "high-contrast",
                if contrast { "false" } else { "true" },
                Some(contrast),
            ),
            (
                "motion",
                "Reduce motion",
                reduced.to_string(),
                "org.gnome.desktop.interface",
                "enable-animations",
                if reduced { "true" } else { "false" },
                Some(reduced),
            ),
            (
                "accent",
                "Accent colour",
                prefs.accent.clone(),
                "org.gnome.desktop.interface",
                "accent-color",
                accent,
                None,
            ),
            (
                "scale",
                "Text size",
                format!("{:.0}%", prefs.scale * 100.),
                "org.gnome.desktop.interface",
                "text-scaling-factor",
                if prefs.scale < 1.24 {
                    "1.25"
                } else if prefs.scale < 1.49 {
                    "1.5"
                } else if prefs.scale < 1.99 {
                    "2.0"
                } else {
                    "1.0"
                },
                None,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut control = if toggle.is_some() {
                gpui::Role::Switch
            } else {
                gpui::Role::Button
            }
            .control(id, label);
            if let Some(on) = toggle {
                control = control.aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                });
            }
            let control = mundane_ui::focus_ring(control, &self.controls[idx], theme::palette())
                .px_3()
                .py_1()
                .flex()
                .justify_between()
                .child(label)
                .child(match toggle {
                    Some(true) => "On".into(),
                    Some(false) => "Off".into(),
                    None => value,
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.spawn(async move |weak, cx| {
                        let error = theme_scan::gsettings_set(schema, key, next)
                            .await
                            .err()
                            .map(|e| e.to_string());
                        weak.update(cx, |this, cx| {
                            this.error = error;
                            cx.notify()
                        })
                        .ok();
                    })
                    .detach();
                }));
            form = form.child(control);
        }
        if let Some(e) = &self.error {
            form = form.child(e.clone());
        }
        form.child(body)
    }
}

fn skeleton() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(skeleton_row("Wallpaper"))
        .child(skeleton_row("GTK theme"))
        .child(skeleton_row("Icon theme"))
        .child(skeleton_row("Interface font"))
        .child(skeleton_row("Monospace font"))
}

fn skeleton_row(label: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(56.))
        .px_4()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .child(
            div()
                .text_color(rgb(theme::palette().secondary))
                .child(label),
        )
        .child(
            div()
                .w(px(180.))
                .h(px(28.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(6.)),
        )
}

fn data_view(
    data: &AppearanceData,
    in_flight: &HashSet<&'static str>,
    open: Option<Section>,
    cx: &mut Context<AppearancePage>,
) -> gpui::Div {
    let mut col = div().flex().flex_col().gap_3();
    col = col.child(picker_row(
        Section::GtkTheme,
        "GTK theme",
        &data.gtk_theme,
        in_flight.contains("gtk_theme"),
        open,
        cx,
    ));
    if open == Some(Section::GtkTheme) {
        col = col.child(theme_list(
            &data.themes_available,
            &data.gtk_theme,
            Section::GtkTheme,
            cx,
        ));
    }
    col = col.child(picker_row(
        Section::IconTheme,
        "Icon theme",
        &data.icon_theme,
        in_flight.contains("icon_theme"),
        open,
        cx,
    ));
    if open == Some(Section::IconTheme) {
        col = col.child(theme_list(
            &data.icons_available,
            &data.icon_theme,
            Section::IconTheme,
            cx,
        ));
    }
    col = col.child(picker_row(
        Section::Wallpaper,
        "Wallpaper",
        &data.wallpaper_uri,
        in_flight.contains("wallpaper"),
        open,
        cx,
    ));
    if open == Some(Section::Wallpaper) {
        col = col.child(wallpaper_list(
            &data.wallpapers_available,
            &data.wallpaper_uri,
            cx,
        ));
    }
    for (section, label, current) in [
        (
            Section::InterfaceFont,
            "Interface font",
            &data.interface_font,
        ),
        (
            Section::MonospaceFont,
            "Monospace font",
            &data.monospace_font,
        ),
    ] {
        col = col.child(picker_row(section, label, current, false, open, cx));
        if open == Some(section) {
            let (family, points) = current
                .rsplit_once(' ')
                .filter(|(_, n)| n.parse::<f64>().is_ok())
                .unwrap_or((current, "11"));
            let names: Vec<_> = data
                .families
                .iter()
                .map(|name| format!("{name} {points}"))
                .collect();
            col = col.child(theme_list(
                &names,
                &format!("{family} {points}"),
                section,
                cx,
            ));
        }
    }
    col
}

fn picker_row(
    section: Section,
    label: &str,
    value: &str,
    in_flight: bool,
    open: Option<Section>,
    cx: &mut Context<AppearancePage>,
) -> AnyElement {
    let chevron = if open == Some(section) { "▾" } else { "▸" };
    let mut row = div()
        .id(SharedString::from(format!("picker:{label}")))
        .role(gpui::Role::Button)
        .aria_label(label.to_string())
        .tab_index(0)
        .focus(|s| s.border_2().border_color(rgb(theme::palette().focus)))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(px(56.))
        .px_4()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::palette().panel)))
        .on_click(cx.listener(move |this, _e, _w, cx| this.toggle_section(section, cx)))
        .child(
            div()
                .text_color(rgb(theme::palette().secondary))
                .child(label.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_color(rgb(theme::palette().text))
                        .child(short_label(value)),
                )
                .child(
                    div()
                        .text_color(rgb(theme::palette().secondary))
                        .child(chevron.to_string()),
                ),
        );
    if in_flight {
        row = row.border_b_2().border_color(rgb(theme::palette().focus));
    }
    row.into_any_element()
}

fn theme_list(
    available: &[String],
    current: &str,
    section: Section,
    cx: &mut Context<AppearancePage>,
) -> gpui::Div {
    let mut col = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .py_2()
        .bg(rgb(theme::palette().surface))
        .rounded(px(10.));
    if available.is_empty() {
        col = col.child(
            div()
                .px_4()
                .py_2()
                .text_color(rgb(theme::palette().secondary))
                .child("No themes found on this system."),
        );
        return col;
    }
    for name in available {
        let is_current = name == current;
        let name_owned = name.clone();
        let row = div()
            .id(SharedString::from(format!("theme-opt:{name}")))
            .role(gpui::Role::Button)
            .aria_label(name.to_string())
            .tab_index(0)
            .focus(|s| s.border_2().border_color(rgb(theme::palette().focus)))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::palette().panel)))
            .on_click(cx.listener(move |this, _e, _w, cx| match section {
                Section::GtkTheme => this.pick_gtk_theme(name_owned.clone(), cx),
                Section::IconTheme => this.pick_icon_theme(name_owned.clone(), cx),
                Section::InterfaceFont => this.set_gsettings(
                    "font-name",
                    "org.gnome.desktop.interface",
                    "font-name",
                    name_owned.clone(),
                    |d, v| d.interface_font = v,
                    cx,
                ),
                Section::MonospaceFont => this.set_gsettings(
                    "monospace-font-name",
                    "org.gnome.desktop.interface",
                    "monospace-font-name",
                    name_owned.clone(),
                    |d, v| d.monospace_font = v,
                    cx,
                ),
                Section::Wallpaper => {}
            }))
            .child(
                div()
                    .text_color(rgb(theme::palette().text))
                    .child(name.clone()),
            )
            .child(
                div()
                    .text_color(rgb(if is_current {
                        theme::palette().focus
                    } else {
                        theme::palette().secondary
                    }))
                    .child(if is_current { "Current" } else { "" }.to_string()),
            );
        col = col.child(row);
    }
    col
}

fn wallpaper_list(
    available: &[PathBuf],
    current_uri: &str,
    cx: &mut Context<AppearancePage>,
) -> gpui::Div {
    let mut col = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .py_2()
        .bg(rgb(theme::palette().surface))
        .rounded(px(10.));
    if available.is_empty() {
        col = col.child(
            div()
                .px_4()
                .py_2()
                .text_color(rgb(theme::palette().secondary))
                .child("No wallpapers in /etc/backgrounds or ~/Pictures."),
        );
        return col;
    }
    for path in available {
        let display = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let uri = format!("file://{}", path.display());
        let is_current = uri == current_uri;
        let path_owned = path.clone();
        let row = div()
            .id(SharedString::from(format!("wp-opt:{}", path.display())))
            .role(gpui::Role::Button)
            .aria_label(display.clone())
            .tab_index(0)
            .focus(|s| s.border_2().border_color(rgb(theme::palette().focus)))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::palette().panel)))
            .on_click(cx.listener(move |this, _e, _w, cx| {
                this.pick_wallpaper(path_owned.clone(), cx);
            }))
            .child(div().text_color(rgb(theme::palette().text)).child(display))
            .child(
                div()
                    .text_color(rgb(if is_current {
                        theme::palette().focus
                    } else {
                        theme::palette().secondary
                    }))
                    .child(if is_current { "Current" } else { "" }.to_string()),
            );
        col = col.child(row);
    }
    col
}

/// Trim a wallpaper URI / theme name for display in the value chip — long
/// `file://...` URIs make the row wrap unattractively.
fn short_label(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("file://") {
        if let Some(name) = std::path::Path::new(rest)
            .file_name()
            .and_then(|n| n.to_str())
        {
            return name.to_string();
        }
    }
    s.to_string()
}
