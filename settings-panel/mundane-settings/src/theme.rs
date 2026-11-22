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
    pub palette: Theme,
    pub dark: bool,
    pub contrast: bool,
    pub reduced: bool,
    pub scale: f64,
    pub font: String,
    pub points: f64,
    pub accent: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            palette: Theme::DARK,
            dark: true,
            contrast: false,
            reduced: false,
            scale: 1.,
            font: "Sans".into(),
            points: 12.,
            accent: "blue".into(),
        }
    }
}

thread_local! {static CURRENT:RefCell<Appearance> = RefCell::new(Appearance::default());}
pub fn current() -> Appearance {
    CURRENT.with_borrow(Clone::clone)
}
pub fn palette() -> Theme {
    CURRENT.with_borrow(|p| p.palette)
}

type Settings = HashMap<String, HashMap<String, OwnedValue>>;
fn read(proxy: &Proxy) -> zbus::Result<Appearance> {
    let all: Settings = proxy.call(
        "ReadAll",
        &(vec![
            "org.freedesktop.appearance",
            "org.gnome.desktop.interface",
        ],),
    )?;
    let value = |group: &str, key: &str| all.get(group).and_then(|values| values.get(key));
    let appearance = "org.freedesktop.appearance";
    let desktop = "org.gnome.desktop.interface";
    let flag = |key: &str| value(appearance, key).and_then(|v| u32::try_from(v).ok()) == Some(1);
    let dark = flag("color-scheme");
    let contrast = flag("contrast");
    let mut palette = if contrast {
        Theme::HIGH_CONTRAST
    } else if dark {
        Theme::DARK
    } else {
        Theme {
            surface: 0xf9faf8,
            panel: 0xffffff,
            text: 0x17201e,
            secondary: 0x394a41,
            border: 0x738279,
            focus: 0x175db1,
            selection: 0xc4dbeb,
        }
    };
    if !contrast {
        if let Some((r, g, b)) = value(appearance, "accent-color")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| <(f64, f64, f64)>::try_from(v).ok())
        {
            if [r, g, b].iter().all(|c| (0.0..=1.0).contains(c)) {
                palette.focus = ((r * 255.).round() as u32) << 16
                    | ((g * 255.).round() as u32) << 8
                    | (b * 255.).round() as u32;
            }
        }
    }
    let scale = value(desktop, "text-scaling-factor")
        .and_then(|v| f64::try_from(v).ok())
        .unwrap_or(1.);
    let font = value(desktop, "font-name")
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
        palette,
        dark,
        contrast,
        reduced: flag("reduced-motion"),
        scale,
        font,
        points,
        accent: value(desktop, "accent-color")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or("blue")
            .to_owned(),
    })
}
pub fn watch(cx: &mut App) {
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
            let mut changes = proxy.receive_signal("SettingChanged")?;
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
