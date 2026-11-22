//! IBus supplies conversion and preedit through ibus-wayland. The shell owns its candidate list.
use crate::{Event, Service, Shell};
use async_channel::Sender;
use gio::{
    glib::{self, Variant},
    prelude::*,
};
use gpui::{
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::*,
    *,
};
use mundane_ui::{tokens, Theme};
use std::{cell::RefCell, rc::Rc, time::Duration};

const PANEL: &str = "org.freedesktop.IBus.Panel";
const PATH: &str = "/org/freedesktop/IBus/Panel";
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Candidates {
    pub words: Vec<String>,
    pub selected: Option<usize>,
    pub aux: String,
}

fn ibus_text(v: &Variant) -> Option<String> {
    if v.type_().as_str() != "(sa{sv}sv)" || v.try_child_get::<String>(0).ok()?? != "IBusText" {
        return None;
    }
    v.try_child_get(2).ok()?
}
fn decode(v: &Variant) -> Option<Candidates> {
    if v.type_().as_str() != "(sa{sv}uubbiavav)"
        || v.try_child_get::<String>(0).ok()?? != "IBusLookupTable"
    {
        return None;
    }
    let size = (v.try_child_get::<u32>(2).ok()?? as usize).clamp(1, 128);
    let cursor = v.try_child_get::<u32>(3).ok()?? as usize;
    let visible = v.try_child_get::<bool>(4).ok()??;
    let all = v.child_value(7);
    let start = (cursor / size) * size;
    let words = (start..(start + size).min(all.n_children()))
        .map(|i| ibus_text(&all.child_value(i).as_variant()?))
        .collect::<Option<Vec<_>>>()?;
    Some(Candidates {
        words,
        selected: visible.then_some(cursor % size),
        aux: String::new(),
    })
}
type Bus = Rc<RefCell<Option<gio::DBusConnection>>>;
pub struct Panel(Bus);
impl Panel {
    pub fn start(tx: Sender<Event>, ctx: &glib::MainContext) -> Self {
        let active = Rc::new(RefCell::new(None));
        if std::env::var_os("MUNDANE_IBUS_GPUI").is_some() {
            let out = active.clone();
            ctx.spawn_local(async move {
                loop {
                    if let Ok(bus) = connect(tx.clone()) {
                        let (send, closed) = async_channel::bounded(1);
                        bus.connect_closed(move |_, _, _| {
                            let _ = send.try_send(());
                        });
                        *out.borrow_mut() = Some(bus.clone());
                        if !bus.is_closed() {
                            let _ = closed.recv().await;
                        }
                        out.borrow_mut().take();
                        let _ = tx.try_send(Event::Candidates(Default::default()));
                    }
                    glib::timeout_future(Duration::from_secs(2)).await;
                }
            });
        }
        Self(active)
    }
    pub fn click(&self, index: u32) {
        if let Some(bus) = self.0.borrow().as_ref() {
            let _ = bus.emit_signal(
                None,
                PATH,
                PANEL,
                "CandidateClicked",
                Some(&(index, 1u32, 0u32).to_variant()),
            );
        }
    }
}
fn connect(tx: Sender<Event>) -> anyhow::Result<gio::DBusConnection> {
    let address = if let Ok(a) = std::env::var("IBUS_ADDRESS") {
        a
    } else {
        let output = std::process::Command::new("ibus").arg("address").output()?;
        anyhow::ensure!(output.status.success(), "IBus address unavailable");
        String::from_utf8(output.stdout)?.trim().to_owned()
    };
    let bus = gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )?;
    bus.set_exit_on_close(false);
    let iface = gio::DBusNodeInfo::for_xml(include_str!("ibus-panel.xml"))?
        .lookup_interface(PANEL)
        .unwrap();
    let state = RefCell::new((Candidates::default(), false, String::new()));
    bus.register_object(PATH, &iface)
        .method_call(move |_, sender, _, _, method, args, reply| {
            // The private IBus driver labels its own panel calls with this name.
            // Other connections receive unique senders from the driver.
            if sender != Some("org.freedesktop.DBus") {
                reply.return_dbus_error(
                    "org.freedesktop.DBus.Error.AccessDenied",
                    "IBus daemon required",
                );
                return;
            }
            let mut state = state.borrow_mut();
            let (last, visible, auxiliary) = &mut *state;
            match method {
                "UpdateLookupTable" => {
                    let aux = std::mem::take(&mut last.aux);
                    *last = decode(&args.child_get(0)).unwrap_or_default();
                    last.aux = aux;
                    *visible = args.child_get(1);
                }
                "HideLookupTable" => *visible = false,
                "ShowLookupTable" => *visible = true,
                "UpdateAuxiliaryText" => {
                    *auxiliary = ibus_text(&args.child_get(0)).unwrap_or_default();
                    last.aux = if args.child_get(1) {
                        auxiliary.clone()
                    } else {
                        String::new()
                    };
                }
                "ShowAuxiliaryText" => last.aux = auxiliary.clone(),
                "HideAuxiliaryText" => last.aux.clear(),
                "Reset" | "FocusOut" | "FocusIn" | "DestroyContext" => {
                    *last = Default::default();
                    *visible = false;
                    auxiliary.clear();
                }
                "ProcessKeyEvent" => {
                    reply.return_value(Some(&(false,).to_variant()));
                    return;
                }
                _ => {
                    reply.return_value(None);
                    return;
                }
            }
            let _ = tx.try_send(Event::Candidates(if *visible {
                last.clone()
            } else {
                Candidates::default()
            }));
            reply.return_value(None);
        })
        .build()?;
    let reply = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(PANEL, 4u32).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
    )?;
    anyhow::ensure!(reply.child_get::<u32>(0) == 1, "IBus panel already owned");
    Ok(bus)
}

pub struct CandidateView {
    names: Candidates,
    focus: Vec<FocusHandle>,
}
impl Render for CandidateView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::current(cx);
        let mut list = div()
            .size_full()
            .flex()
            .flex_col()
            .p(tokens::space::XS)
            .rounded(tokens::radius::CONTROL)
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.panel))
            .text_color(rgb(t.text))
            .text_size(tokens::text::BODY);
        if !self.names.aux.is_empty() {
            list = list.child(self.names.aux.clone());
        }
        let tx = cx.global::<Shell>().service.clone();
        for (i, name) in self.names.words.iter().enumerate() {
            if self.focus.len() <= i {
                self.focus.push(cx.focus_handle());
            }
            let tx = tx.clone();
            list = list.child(
                mundane_ui::launcher_row(
                    name.clone(),
                    &self.focus[i],
                    Some(i) == self.names.selected,
                    true,
                    t,
                    move |_, _| {
                        let _ = tx.try_send(Service::Candidate(i as u32));
                    },
                )
                .child(format!("{}    {}", i + 1, name)),
            );
        }
        list
    }
}
pub fn redraw(cx: &mut App) {
    let s = cx.global::<Shell>();
    let caret = s
        .caret
        .filter(|_| !s.locked && s.auth.is_none() && !s.candidates.words.is_empty());
    let placement = caret.and_then(|[x, y, _, height]| {
        let d = cx
            .displays()
            .into_iter()
            .find(|d| d.bounds().contains(&point(px(x as f32), px(y as f32))))?;
        let b = d.bounds();
        let w = 240f32.min(b.size.width.into());
        let h = ((s.candidates.words.len() as f32 + (!s.candidates.aux.is_empty()) as u8 as f32)
            * 34.
            + 12.)
            .min(b.size.height.into());
        let left = (x as f32 - f32::from(b.origin.x)).clamp(0., f32::from(b.size.width) - w);
        let above = y as f32 - f32::from(b.origin.y);
        let below = above + height as f32;
        let top = if below + h <= b.size.height.into() {
            below
        } else {
            (above - h).max(0.)
        };
        Some((d.id(), left as i32, top as i32, w as i32, h as i32))
    });
    if let Some((old, win)) = &s.candidate_window {
        if placement == Some(*old) {
            let win = *win;
            let names = s.candidates.clone();
            let _ = win.update(cx, |v, _, cx| {
                v.names = names;
                cx.notify();
            });
            return;
        }
    }
    if let Some((_, win)) = cx.global_mut::<Shell>().candidate_window.take() {
        let _ = win.update(cx, |_, w, _| w.remove_window());
    }
    if let Some((display, x, y, w, h)) = placement {
        let names = cx.global::<Shell>().candidates.clone();
        if let Ok(win) = cx.open_window(
            WindowOptions {
                display_id: Some(display),
                focus: false,
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(w as f32), px(h as f32)),
                ))),
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: "mundane-input-candidates".into(),
                    layer: Layer::Overlay,
                    anchor: Anchor::TOP | Anchor::LEFT,
                    margin: Some((px(y as f32), px(0.), px(0.), px(x as f32))),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    exclusive_zone: Some(px(-1.)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |w, cx| {
                w.set_window_title("Input method candidates");
                cx.new(|_| CandidateView {
                    names,
                    focus: vec![],
                })
            },
        ) {
            cx.global_mut::<Shell>().candidate_window = Some(((display, x, y, w, h), win));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, ibus_text};
    use gio::glib::{variant::ToVariant, Variant};
    use std::collections::BTreeMap;

    #[test]
    fn lookup_table_exposes_only_the_engine_page() {
        let props = BTreeMap::<String, Variant>::new();
        let text = |s: &str| ("IBusText", &props, s, 0u32.to_variant()).to_variant();
        let table = (
            "IBusLookupTable",
            &props,
            2u32,
            2u32,
            true,
            false,
            0i32,
            vec![text("one"), text("two"), text("three")],
            Vec::<Variant>::new(),
        )
            .to_variant();
        let panel = decode(&table).unwrap();
        assert_eq!(panel.words, ["three"]);
        assert_eq!(panel.selected, Some(0));
        assert!(decode(&"wrong type".to_variant()).is_none());
        assert!(ibus_text(&false.to_variant()).is_none());
    }
}
