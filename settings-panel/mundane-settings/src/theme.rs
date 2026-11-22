//! Apply the desktop Settings portal's initial preferences and live changes.
use gpui::App;
use mundane_ui::Theme;
use std::cell::RefCell;
use std::collections::HashMap;
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedValue,
};

#[derive(Clone)]
pub struct Appearance {
    pub scale: f64,
    pub font: String,
    pub points: f64,
    pub accent: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            scale: 1.,
            font: "Sans".into(),
            points: 12.,
            accent: "blue".into(),
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Appearance> = RefCell::new(Appearance::default());
    // Stateless row builders use the same palette as the shared GPUI controls.
    static PALETTE: std::cell::Cell<Theme> = const { std::cell::Cell::new(Theme::DARK) };
}
pub fn current() -> Appearance {
    CURRENT.with_borrow(Clone::clone)
}
pub fn palette() -> Theme {
    PALETTE.get()
}

type Settings = HashMap<String, HashMap<String, OwnedValue>>;
fn read(proxy: &Proxy) -> zbus::Result<Appearance> {
    let desktop = "org.gnome.desktop.interface";
    let all: Settings = proxy.call("ReadAll", &([desktop].as_slice(),))?;
    let value = |key: &str| all.get(desktop).and_then(|values| values.get(key));
    let scale = value("text-scaling-factor")
        .and_then(|v| f64::try_from(v).ok())
        .unwrap_or(1.);
    let font = value("font-name")
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("Sans 11");
    let (family, points) = font
        .rsplit_once(' ')
        .and_then(|(f, n)| {
            n.parse::<f64>()
                .ok()
                .filter(|s| s.is_finite() && *s > 0.)
                .map(|n| (f, n))
        })
        .unwrap_or((font, 12.));
    let font = family.to_owned();
    Ok(Appearance {
        scale,
        font,
        points,
        accent: value("accent-color")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or("blue")
            .to_owned(),
    })
}
pub fn watch(cx: &mut App) {
    PALETTE.set(Theme::current(cx));
    cx.observe_global::<Theme>(|cx| PALETTE.set(Theme::current(cx)))
        .detach();
    let (tx, rx) = async_channel::bounded(8);
    std::thread::spawn(move || {
        let run = || -> zbus::Result<()> {
            let bus = Connection::session()?;
            let proxy = Proxy::new(
                &bus,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            )?;
            let mut changes = proxy.receive_signal_with_args(
                "SettingChanged",
                &[(0, "org.gnome.desktop.interface")],
            )?;
            loop {
                if tx.send_blocking(read(&proxy)?).is_err() || changes.next().is_none() {
                    return Ok(());
                }
            }
        };
        while !tx.is_closed() {
            if let Err(err) = run() {
                tracing::warn!(%err,"reconnecting to appearance portal");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    cx.spawn(async move |cx| {
        while let Ok(next) = rx.recv().await {
            CURRENT.with_borrow_mut(|p| *p = next);
            cx.refresh();
        }
    })
    .detach();
}
