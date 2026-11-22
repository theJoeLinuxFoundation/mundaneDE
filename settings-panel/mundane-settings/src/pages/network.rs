//! Network page. Lists Wi-Fi networks and known VPN connections; clicking a
//! Wi-Fi row launches a connect flow (password prompt for secured nets) via
//! `nmcli`.

use crate::pages::{Page, PageState};
use crate::theme;
use crate::widgets::prompt;
use gpui::{
    div, px, rgb, AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use mundane_settings_core::dbus::network as backend;

#[derive(Default, Clone)]
pub struct NetworkData {
    pub wifi: Vec<backend::WifiNetwork>,
    pub vpn: Vec<backend::VpnConnection>,
}

pub struct NetworkPage {
    state: PageState<NetworkData>,
    connecting: Option<String>,
    last_error: Option<String>,
}

impl NetworkPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = Self {
            state: PageState::Empty,
            connecting: None,
            last_error: None,
        };
        s.spawn_refresh(cx);
        s
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, async_cx| {
            let wifi = backend::list_wifi().await.unwrap_or_default();
            let vpn = backend::list_vpn().await.unwrap_or_default();
            weak.update(async_cx, |this, cx| {
                this.state = PageState::Loaded {
                    data: NetworkData { wifi, vpn },
                    fetched_at: std::time::Instant::now(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_connect(&mut self, ssid: String, secured: bool, cx: &mut Context<Self>) {
        if self.connecting.is_some() {
            return;
        }
        self.connecting = Some(ssid.clone());
        self.last_error = None;
        cx.notify();
        cx.spawn(async move |weak, async_cx| {
            let password = if secured {
                match prompt::password("Connect to Wi-Fi", &format!("Password for {ssid}")).await {
                    Ok(Some(p)) if !p.is_empty() => Some(p),
                    Ok(_) => {
                        weak.update(async_cx, |this, cx| {
                            this.connecting = None;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                    Err(e) => {
                        weak.update(async_cx, |this, cx| {
                            this.connecting = None;
                            this.last_error = Some(e.to_string());
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                }
            } else {
                None
            };
            let result = backend::connect_wifi(&ssid, password.as_deref()).await;
            weak.update(async_cx, |this, cx| {
                this.connecting = None;
                if let Err(e) = result {
                    this.last_error = Some(e.to_string());
                } else {
                    this.last_error = None;
                }
                cx.notify();
                this.spawn_refresh(cx);
            })
            .ok();
        })
        .detach();
    }
}

impl Render for NetworkPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.state.should_refresh(Page::Network.ttl()) {
            self.spawn_refresh(cx);
        }
        let connecting = self.connecting.clone();
        let error = self.last_error.clone();
        let body: AnyElement = match &self.state {
            PageState::Loaded { data, .. } => {
                data_view(data, connecting.as_deref(), error.as_deref(), cx).into_any_element()
            }
            _ => skeleton().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .child(div().text_xl().pb_4().child("Network"))
            .child(body)
    }
}

fn skeleton() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .h(px(56.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
        .child(
            div()
                .h(px(56.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
        .child(
            div()
                .h(px(56.))
                .bg(rgb(theme::palette().panel))
                .rounded(px(10.)),
        )
}

fn data_view(
    data: &NetworkData,
    connecting: Option<&str>,
    error: Option<&str>,
    cx: &mut Context<NetworkPage>,
) -> gpui::Div {
    let mut col = div().flex().flex_col().gap_4();
    col = col.child(section_header("Wi-Fi"));
    if data.wifi.is_empty() {
        col = col.child(empty_card(
            "No wireless networks visible. (Wi-Fi off, or no NetworkManager-managed adapter.)",
        ));
    } else {
        for w in &data.wifi {
            col = col.child(wifi_row(w, connecting, cx));
        }
    }
    if let Some(e) = error {
        col = col.child(
            div()
                .px_4()
                .py_2()
                .text_color(rgb(theme::palette().secondary))
                .child(format!("Couldn't connect: {e}")),
        );
    }
    col = col.child(section_header("VPN"));
    if data.vpn.is_empty() {
        col = col.child(empty_card("No VPN connections configured."));
    } else {
        for v in &data.vpn {
            col = col.child(vpn_row(v));
        }
    }
    col
}

fn section_header(label: &'static str) -> gpui::Div {
    div()
        .text_color(rgb(theme::palette().secondary))
        .pt_2()
        .child(label)
}

fn empty_card(message: &str) -> gpui::Div {
    div()
        .h(px(60.))
        .px_4()
        .flex()
        .items_center()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .text_color(rgb(theme::palette().secondary))
        .child(message.to_string())
}

fn wifi_row(
    w: &backend::WifiNetwork,
    connecting: Option<&str>,
    cx: &mut Context<NetworkPage>,
) -> AnyElement {
    let ssid_for_listener = w.ssid.clone();
    let secured = w.secured;
    let is_connecting = connecting == Some(w.ssid.as_str());
    let any_pending = connecting.is_some();
    let chip = if is_connecting {
        Some(("Connecting…", theme::palette().focus))
    } else if w.connected {
        Some(("Connected", theme::palette().focus))
    } else if w.secured {
        Some(("Secured", theme::palette().secondary))
    } else {
        None
    };
    let mut row = div()
        .id(SharedString::from(format!("wifi-row:{}", w.ssid)))
        .role(gpui::Role::Button)
        .aria_label(w.ssid.clone())
        .tab_index(0)
        .focus(|s| s.border_2().border_color(rgb(theme::palette().focus)))
        .flex()
        .flex_col()
        .gap_1()
        .px_4()
        .py_3()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::palette().panel)))
        .on_click(cx.listener(move |this, _e, _w, cx| {
            this.start_connect(ssid_for_listener.clone(), secured, cx);
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_color(rgb(theme::palette().text))
                        .child(w.ssid.clone()),
                )
                .child(
                    div()
                        .text_color(rgb(theme::palette().secondary))
                        .child(format!("{}%", w.strength)),
                ),
        );
    if let Some((label, color)) = chip {
        row = row.child(div().text_color(rgb(color)).child(label.to_string()));
    }
    if is_connecting {
        row = row.border_b_2().border_color(rgb(theme::palette().focus));
    }
    if any_pending && !is_connecting {
        // Subdue other rows visually while a connect is in flight.
        row = row.text_color(rgb(theme::palette().secondary));
    }
    row.into_any_element()
}

fn vpn_row(v: &backend::VpnConnection) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .bg(rgb(theme::palette().panel))
        .rounded(px(10.))
        .child(div().child(v.name.clone()))
        .child(
            div()
                .text_color(if v.active {
                    rgb(theme::palette().focus)
                } else {
                    rgb(theme::palette().secondary)
                })
                .child(if v.active { "active" } else { "inactive" }.to_string()),
        )
}
