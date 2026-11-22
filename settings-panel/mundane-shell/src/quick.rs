//! The Quick Settings view owns focus and dialog lifetimes, never device state.
use crate::{
    devices::{Action, Request},
    Event, Page, Service, Shell,
};
use gpui::{prelude::*, *};
use mundane_ui::{
    tokens, ApplicationContent, ConfirmDialog, Confirmation, PanelState, SliderRange, TextField,
    Theme, TileState, ValueSlider,
};
use std::collections::HashMap;
pub struct Quick {
    display: DisplayId,
    focus: HashMap<String, FocusHandle>,
    volume: ValueSlider,
    brightness: ValueSlider,
    expanded: bool,
    scroll: ScrollHandle,
    confirmation: Option<Entity<ConfirmDialog>>,
    content: ApplicationContent,
    pairing: Option<(u64, Entity<TextField>)>,
    first: bool,
}
impl Quick {
    pub fn new(display: DisplayId, cx: &App) -> Self {
        Self {
            display,
            focus: Default::default(),
            volume: ValueSlider::new(cx),
            brightness: ValueSlider::new(cx),
            expanded: false,
            scroll: ScrollHandle::new(),
            confirmation: None,
            content: Default::default(),
            pairing: None,
            first: true,
        }
    }
    fn focus(&mut self, name: &str, cx: &App) -> FocusHandle {
        self.focus
            .entry(name.to_owned())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }
    pub fn escape(&mut self, w: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(d) = &self.confirmation {
            d.update(cx, |d, cx| d.finish(Confirmation::Cancelled, w, cx));
            return true;
        }
        if cx.global::<Shell>().pairing.is_some() {
            let _ = cx.global::<Shell>().service.try_send(Service::CancelPair);
            return true;
        }
        false
    }
    fn confirm(
        &mut self,
        name: &'static str,
        op: &'static str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| {
            ConfirmDialog::new(
                format!("{name} this computer?"),
                "Open applications will close. Save your work first.",
                name,
                cx,
            )
        });
        let trigger = self.focus(name, cx);
        cx.subscribe(&dialog, move |v, _, answer, cx| {
            v.confirmation = None;
            if *answer == Confirmation::Confirmed {
                let _ = cx
                    .global::<Shell>()
                    .device_tx
                    .try_send(Request::Apply(Action::Power(op)));
            }
            cx.notify();
        })
        .detach();
        dialog.update(cx, |d, cx| d.show(&trigger, w, cx));
        self.confirmation = Some(dialog);
        cx.notify();
    }
}
// AccessKit can activate a clipped control. Dispatch by identity and reveal
// keyboard focus inside the scroll area, which ends above the power row.
fn reveal(child: impl IntoElement, focus: FocusHandle, scroll: &ScrollHandle) -> Div {
    let scroll = scroll.clone();
    div()
        .flex_none()
        .on_children_prepainted(move |bounds, w, _| {
            if let Some(item) = bounds.first() {
                crate::view::reveal_focus(*item, &focus, &scroll, w);
            }
        })
        .child(child)
}

fn text_button(
    label: &'static str,
    focus: &FocusHandle,
    theme: Theme,
    action: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    mundane_ui::button(label, focus, theme, action.clone())
        .on_a11y_action(AccessibleAction::Click, move |_, w, cx| action(w, cx))
}

fn icon(name: &str, fallback: &str, ink: u32) -> AnyElement {
    match crate::catalog::icon_path(name, "Adwaita") {
        Some(p) if p.extension().is_some_and(|e| e == "svg") => svg()
            .external_path(p.to_string_lossy().into_owned())
            .text_color(rgb(ink))
            .size(tokens::icon::LARGE)
            .into_any_element(),
        Some(p) => img(p).size(tokens::icon::LARGE).into_any_element(),
        None => div().child(fallback.to_owned()).into_any_element(),
    }
}
fn tile(
    name: &'static str,
    symbol: &'static str,
    value: Option<bool>,
    missing: &str,
    focus: &FocusHandle,
    action: fn(bool) -> Action,
    cx: &App,
) -> Stateful<Div> {
    let tx = cx.global::<Shell>().device_tx.clone();
    let accessible = tx.clone();
    mundane_ui::quick_tile(
        TileState {
            label: name,
            status: match value {
                Some(true) => "On",
                Some(false) => "Off",
                None => missing,
            },
            enabled: value.is_some(),
            on: value == Some(true),
            error: None,
        },
        icon(
            symbol,
            "",
            if value.is_some() {
                Theme::current(cx).text
            } else {
                Theme::current(cx).disabled()
            },
        ),
        focus,
        Theme::current(cx),
        move |v, _, _| {
            let _ = tx.try_send(Request::Apply(action(v)));
        },
    )
    .when_some(value, |el, enabled| {
        el.on_a11y_action(AccessibleAction::Click, move |_, _, _| {
            let _ = accessible.try_send(Request::Apply(action(!enabled)));
        })
    })
    .flex_1()
}
impl Render for Quick {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(cx);
        let shell = cx.global::<Shell>();
        let d = shell.devices.clone();
        let device_tx = shell.device_tx.clone();
        let events = shell.events.clone();
        let service = shell.service.clone();
        let mut header = div().flex().items_center().justify_between().child(
            div()
                .flex()
                .flex_col()
                .child(
                    mundane_ui::heading("quick-title", "Quick Settings")
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(tokens::text::TITLE),
                )
                .when_some(
                    gio::glib::DateTime::now_local()
                        .ok()
                        .and_then(|d| d.format("%A, %B %-d").ok()),
                    |el, date| {
                        let date = date.to_string();
                        el.child(
                            mundane_ui::text_label("quick-date", date.clone())
                                .text_size(tokens::text::CAPTION)
                                .text_color(rgb(theme.muted()))
                                .child(date),
                        )
                    },
                ),
        );
        if let Some(battery) = &d.battery {
            header = header.child(mundane_ui::badge(
                "battery",
                "Battery charge",
                battery.clone(),
                theme,
            ));
        }
        let wifi = self.focus("Wi-Fi", cx);
        let bt = self.focus("Bluetooth", cx);
        let dnd = self.focus("Do Not Disturb", cx);
        let mut body = div()
            .id("quick-scroll")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(tokens::space::SM)
            .child(header)
            .child(
                div()
                    .flex()
                    .gap(tokens::space::SM)
                    .child(tile(
                        "Wi-Fi",
                        "network-wireless-symbolic",
                        d.wifi,
                        "Unavailable",
                        &wifi,
                        Action::Wifi,
                        cx,
                    ))
                    .child(tile(
                        "Bluetooth",
                        "bluetooth-active-symbolic",
                        d.bluetooth,
                        "No adapter",
                        &bt,
                        Action::Bluetooth,
                        cx,
                    )),
            )
            .child(
                tile(
                    "Do Not Disturb",
                    "notifications-disabled-symbolic",
                    Some(d.quiet),
                    "",
                    &dnd,
                    Action::Quiet,
                    cx,
                )
                .flex_none(),
            );
        let range = SliderRange::new(0., 100., 1.).unwrap();
        let volume = device_tx.clone();
        body = body.child(self.volume.render(
            "Volume",
            range,
            f64::from(d.volume.unwrap_or(0)),
            d.volume.is_some(),
            theme,
            move |v, _, _| {
                let _ = volume.try_send(Request::Apply(Action::Volume(v as u32)));
            },
        ));
        if d.muted || d.volume.is_none() {
            body = body.child(if d.muted {
                "Muted"
            } else {
                "Audio output unavailable"
            });
        }
        if let Some(b) = d.brightness {
            let tx = device_tx.clone();
            body = body.child(self.brightness.render(
                "Brightness",
                range,
                f64::from(b.percent),
                true,
                theme,
                move |v, _, _| {
                    let _ = tx.try_send(Request::Apply(Action::Brightness(v as u32)));
                },
            ));
        }
        let display = self.display;
        if !shell.tray.is_empty() {
            let mut items = div().flex().flex_wrap().gap(tokens::space::XS);
            for i in &shell.tray {
                let ev = if i.menu.is_empty() {
                    Event::Service(Service::TrayActivate(i.key.clone()))
                } else {
                    Event::Open(display, Page::Tray(i.key.clone(), vec![0]))
                };
                let tx = events.clone();
                items = items.child(
                    crate::view::shell_button(
                        i.title.clone(),
                        &self.focus(&i.key, cx),
                        theme,
                        move |_, _| {
                            let _ = tx.try_send(ev.clone());
                        },
                    )
                    .px(tokens::space::SM)
                    .child(i.title.clone()),
                );
            }
            body = body.child(items);
        }
        if d.bluetooth.is_some()
            || !d.networks.is_empty()
            || !d.sinks.is_empty()
            || !d.peers.is_empty()
        {
            let weak = cx.weak_entity();
            let f = self.focus("details", cx);
            body = body.child(reveal(
                text_button("Devices and connections", &f, theme, move |_, cx| {
                    let _ = weak.update(cx, |q, cx| {
                        q.expanded = !q.expanded;
                        cx.notify();
                    });
                })
                .aria_expanded(self.expanded),
                f,
                &self.scroll,
            ));
        }
        if self.expanded {
            let actions = d
                .sinks
                .into_iter()
                .map(|(id, label)| (label, Action::Sink(id)))
                .chain(
                    d.networks
                        .into_iter()
                        .map(|n| (n.clone(), Action::Network(n))),
                )
                .chain(
                    d.bluetooth
                        .map(|_| ("Discover Bluetooth devices".into(), Action::Scan)),
                )
                .chain(
                    d.peers
                        .iter()
                        .map(|(p, n)| (format!("Connect {n}"), Action::Connect(p.clone()))),
                );
            for (label, action) in actions {
                let tx = device_tx.clone();
                let f = self.focus(&label, cx);
                body = body.child(reveal(
                    crate::view::shell_button(label.clone(), &f, theme, move |_, _| {
                        let _ = tx.try_send(Request::Apply(action.clone()));
                    })
                    .child(label),
                    f,
                    &self.scroll,
                ));
            }
            for (path, name) in d.peers {
                let label = format!("Pair {name}");
                let tx = service.clone();
                let f = self.focus(&label, cx);
                body = body.child(reveal(
                    crate::view::shell_button(label.clone(), &f, theme, move |_, _| {
                        let _ = tx.try_send(Service::Pair(path.clone(), name.clone()));
                    })
                    .child(label),
                    f,
                    &self.scroll,
                ));
            }
        }
        let open = events.clone();
        let history = format!("Notifications ({})", shell.notes.len());
        let f = self.focus("history", cx);
        body = body.child(reveal(
            crate::view::shell_button(history.clone(), &f, theme, move |_, _| {
                let _ = open.try_send(Event::Open(display, Page::History));
            })
            .px(tokens::space::MD)
            .py(tokens::space::XS)
            .child(history),
            f,
            &self.scroll,
        ));
        for (n, detail) in shell.upgrades.iter().enumerate() {
            body = body.child(
                mundane_ui::text_label(("deployment", n), detail.clone())
                    .text_size(tokens::text::CAPTION)
                    .child(detail.clone()),
            );
        }
        if let Some(err) = d.error {
            body = body.child(
                mundane_ui::text_label("device-error", err.clone())
                    .text_color(rgb(theme.error()))
                    .child(err),
            );
        }
        if !shell.device_message.is_empty() {
            body = body.child(
                mundane_ui::text_label("device-message", shell.device_message.clone())
                    .child(shell.device_message.clone()),
            );
        }
        let mut power = div().flex().justify_between().gap(tokens::space::XS);
        for (name, symbol, op) in [
            ("Lock", "system-lock-screen-symbolic", "lock"),
            ("Log Out", "system-log-out-symbolic", "logout"),
            ("Suspend", "weather-clear-night-symbolic", "suspend"),
            ("Restart", "system-reboot-symbolic", "reboot"),
            ("Shut Down", "system-shutdown-symbolic", "poweroff"),
        ] {
            let trigger = self.focus(name, cx);
            let view = cx.weak_entity();
            let tx = events.clone();
            let devices = device_tx.clone();
            let button = mundane_ui::panel_button(
                name,
                icon(symbol, name, theme.text),
                &trigger,
                PanelState::default(),
                theme,
                move |w, cx| match op {
                    "lock" | "logout" => {
                        let _ = tx.try_send(Event::Command(op.into()));
                    }
                    "suspend" => {
                        let _ = devices.try_send(Request::Apply(Action::Power(op)));
                    }
                    _ => {
                        let _ = view.update(cx, |q, cx| q.confirm(name, op, w, cx));
                    }
                },
            );
            power = power.child(mundane_ui::with_tooltip(button, name).flex_1());
        }
        let prompt = shell.pairing.clone();
        let covered = self.confirmation.is_some() || prompt.is_some();
        let mut root = div().relative().size_full().child(
            self.content
                .render(covered)
                .size_full()
                .flex()
                .flex_col()
                .gap(tokens::space::MD)
                .child(body)
                .child(power),
        );
        if let Some(dialog) = &self.confirmation {
            root = root.child(dialog.clone());
        }
        if let Some(p) = prompt {
            let input = matches!(p.method.as_str(), "RequestPinCode" | "RequestPasskey");
            if self.pairing.as_ref().is_none_or(|(id, _)| *id != p.serial) {
                self.pairing = Some((
                    p.serial,
                    cx.new(|cx| TextField::new("Device passkey", theme, cx)),
                ));
                self.focus("Cancel pairing", cx).focus(w, cx);
            }
            let field = self.pairing.as_ref().unwrap().1.clone();
            let tx = service.clone();
            let mut form = div()
                .flex()
                .flex_col()
                .gap(tokens::space::MD)
                .p(tokens::space::LG)
                .child(mundane_ui::heading("pair-heading", "Bluetooth pairing"))
                .child(mundane_ui::text_label("pair-detail", p.text.clone()).child(p.text));
            if input {
                form = form.child(field.clone());
            }
            form = form.child(text_button(
                "Cancel pairing",
                &self.focus("Cancel pairing", cx),
                theme,
                move |_, _| {
                    let _ = tx.try_send(Service::CancelPair);
                },
            ));
            if !p.method.starts_with("Display") {
                form = form.child(text_button(
                    "Confirm",
                    &self.focus("Confirm pairing", cx),
                    theme,
                    move |_, cx| {
                        let _ = service
                            .try_send(Service::PairAnswer(p.serial, field.read(cx).text().into()));
                    },
                ));
            }
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(rgb(theme.panel))
                    .child(form),
            );
        } else {
            self.pairing = None;
        }
        if self.first {
            dnd.focus(w, cx);
            self.first = false;
        }
        root
    }
}
