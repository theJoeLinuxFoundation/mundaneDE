//! Session-user device actions. The worker reads on menu open, service signals and writes.
use crate::Event;
use anyhow::{anyhow, ensure, Result};
use async_channel::{Receiver, Sender};
use gio::{
    glib::{self, Variant},
    prelude::*,
};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Devices {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
    pub quiet: bool,
    pub volume: Option<u32>,
    pub muted: bool,
    pub brightness: Option<Backlight>,
    pub battery: Option<String>,
    pub networks: Vec<String>,
    pub sinks: Vec<(String, String)>,
    pub peers: Vec<(String, String)>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Backlight {
    pub name: String,
    pub max: u32,
    pub percent: u32,
}
impl Backlight {
    fn units(&self, percent: u32) -> u32 {
        ((u64::from(self.max) * u64::from(percent.min(100)) + 50) / 100) as u32
    }
}
#[derive(Clone)]
pub enum Action {
    Wifi(bool),
    Bluetooth(bool),
    Quiet(bool),
    Volume(u32),
    Brightness(u32),
    Network(String),
    Sink(String),
    Scan,
    Connect(String),
    Power(&'static str),
}
#[derive(Clone)]
pub enum Request {
    MediaKey(String),
    Visible(bool),
    Changed,
    Apply(Action),
}
fn run(bin: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(bin)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()?;
    ensure!(
        out.status.success(),
        "{bin}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}
fn read_backlight(root: &Path) -> Option<Backlight> {
    let mut entries: Vec<_> = fs::read_dir(root).ok()?.flatten().collect();
    entries.sort_by_key(|d| d.file_name());
    entries.into_iter().find_map(|entry| {
        let read = |file| {
            fs::read_to_string(entry.path().join(file))
                .ok()?
                .trim()
                .parse::<u32>()
                .ok()
        };
        let (max, value) = (read("max_brightness")?, read("brightness")?);
        (max > 0).then(|| Backlight {
            name: entry.file_name().to_string_lossy().into(),
            max,
            percent: ((100. * f64::from(value) / f64::from(max)).round() as u32).min(100),
        })
    })
}
pub(super) fn call(
    bus: &gio::DBusConnection,
    dest: &str,
    path: &str,
    iface: &str,
    member: &str,
    args: Option<&Variant>,
) -> Result<Variant> {
    Ok(bus.call_sync(
        Some(dest),
        path,
        iface,
        member,
        args,
        None,
        gio::DBusCallFlags::NONE,
        2500,
        gio::Cancellable::NONE,
    )?)
}
type Properties = HashMap<String, Variant>;
fn prop<T: glib::variant::FromVariant>(props: &Properties, key: &str) -> Option<T> {
    props.get(key)?.get()
}
fn snapshot(bus: Option<&gio::DBusConnection>, notifications: &gio::Settings) -> Devices {
    let mut d = Devices {
        quiet: !notifications.boolean("show-banners"),
        brightness: read_backlight(Path::new("/sys/class/backlight")),
        ..Default::default()
    };
    if let Some(bus) = bus {
        if let Ok(r) = call(
            bus,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&("org.freedesktop.UPower.Device",).to_variant()),
        ) {
            let p: Properties = r.child_get(0);
            if prop(&p, "IsPresent") == Some(true) {
                d.battery = prop::<f64>(&p, "Percentage").map(|p| format!("Battery {p:.0}%"));
            }
        }
        if let Ok(r) = call(
            bus,
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None,
        ) {
            let objects: HashMap<glib::variant::ObjectPath, HashMap<String, Properties>> =
                r.child_get(0);
            for (path, ifaces) in objects {
                if let Some(a) = ifaces.get("org.bluez.Adapter1") {
                    d.bluetooth =
                        Some(d.bluetooth.unwrap_or(false) || prop(a, "Powered").unwrap_or(false));
                }
                if let Some(p) = ifaces.get("org.bluez.Device1") {
                    if let Some(name) =
                        prop::<String>(p, "Alias").or_else(|| prop::<String>(p, "Address"))
                    {
                        d.peers.push((path.to_string(), name));
                    }
                }
            }
            d.peers.sort();
        }
    }
    d.wifi = match run("nmcli", &["-w", "2", "radio", "wifi"]).as_deref() {
        Ok("enabled") => Some(true),
        Ok("disabled") => Some(false),
        _ => None,
    };
    d.networks = run(
        "nmcli",
        &[
            "-w",
            "2",
            "-t",
            "--escape",
            "no",
            "-f",
            "NAME",
            "connection",
            "show",
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::to_owned)
    .collect();
    let volume = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).unwrap_or_default();
    d.volume = volume
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| (v * 100.).round() as u32);
    d.muted = volume.contains("[MUTED]");
    if let Ok(sinks) = run("pactl", &["--format=json", "list", "sinks"]) {
        if let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&sinks) {
            d.sinks = list
                .iter()
                .filter_map(|s| {
                    Some((
                        s["name"].as_str()?.into(),
                        s["description"].as_str()?.into(),
                    ))
                })
                .collect();
        }
    }
    d
}
fn apply(
    a: Action,
    d: &Devices,
    bus: Option<&gio::DBusConnection>,
    prefs: &gio::Settings,
) -> Result<()> {
    let on = |b| if b { "on" } else { "off" };
    match a {
        Action::Wifi(v) => {
            run("nmcli", &["-w", "10", "radio", "wifi", on(v)])?;
        }
        Action::Quiet(v) => prefs.set_boolean("show-banners", !v)?,
        Action::Bluetooth(v) => {
            run("bluetoothctl", &["--timeout", "5", "power", on(v)])?;
        }
        Action::Volume(v) => {
            run(
                "wpctl",
                &[
                    "set-volume",
                    "-l",
                    "1.0",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{}%", v.min(100)),
                ],
            )?;
        }
        Action::Brightness(v) => {
            let b = d
                .brightness
                .as_ref()
                .ok_or_else(|| anyhow!("Backlight disconnected"))?;
            call(
                bus.ok_or_else(|| anyhow!("System bus unavailable"))?,
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                Some(&("backlight", &b.name, b.units(v)).to_variant()),
            )?;
        }
        Action::Network(n) => {
            run("nmcli", &["-w", "10", "connection", "up", "id", &n])?;
        }
        Action::Sink(s) => {
            run("pactl", &["set-default-sink", &s])?;
        }
        Action::Connect(path) => {
            call(
                bus.ok_or_else(|| anyhow!("System bus unavailable"))?,
                "org.bluez",
                &path,
                "org.bluez.Device1",
                "Connect",
                None,
            )?;
        }
        Action::Scan => {
            run("bluetoothctl", &["--timeout", "5", "scan", "on"])?;
        }
        Action::Power(op) => {
            run("systemctl", &[op])?;
        }
    }
    Ok(())
}
fn media_key(
    key: &str,
    bus: Option<&gio::DBusConnection>,
    prefs: &gio::Settings,
) -> Result<(&'static str, u8, bool)> {
    let root = Path::new("/sys/class/backlight");
    if matches!(key, "brightness-up" | "brightness-down") {
        let b = read_backlight(root).ok_or_else(|| anyhow!("No backlight"))?;
        let value = if key == "brightness-up" {
            (b.percent + 5).min(100)
        } else {
            b.percent.saturating_sub(5)
        };
        apply(
            Action::Brightness(value),
            &Devices {
                brightness: Some(b),
                ..Default::default()
            },
            bus,
            prefs,
        )?;
        return Ok((
            "Brightness",
            read_backlight(root)
                .ok_or_else(|| anyhow!("Backlight disconnected"))?
                .percent as u8,
            false,
        ));
    }
    match key {
        "mute" => {
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])?;
        }
        "5%+" | "5%-" => {
            run(
                "wpctl",
                &["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", key],
            )?;
        }
        _ => anyhow::bail!("Unknown media key"),
    }
    let v = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?;
    let level = v
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("Missing sink level"))?
        .parse::<f64>()?;
    Ok((
        "Volume",
        (level * 100.).round().clamp(0., 100.) as u8,
        v.contains("[MUTED]"),
    ))
}
fn audio_monitor(tx: Sender<Request>) -> Option<Child> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("pactl");
    let parent = std::process::id();
    // The subscription must end even if the compositor kills this shell while the menu is open.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() as u32 != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    let mut child = cmd
        .arg("subscribe")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if line.contains("on sink #") || line.contains("on server #") {
                let _ = tx.try_send(Request::Changed);
            }
        }
    });
    Some(child)
}
pub fn start(events: Sender<Event>) -> Sender<Request> {
    let (tx, rx) = async_channel::unbounded();
    let sender = tx.clone();
    std::thread::spawn(move || worker(rx, sender, events));
    tx
}
fn worker(rx: Receiver<Request>, tx: Sender<Request>, events: Sender<Event>) {
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).ok();
    let prefs = gio::Settings::new("org.gnome.desktop.notifications");
    let mut active = false;
    let mut audio = None;
    let mut d = Devices::default();
    while let Ok(first) = rx.recv_blocking() {
        let mut error = None;
        let mut dirty = false;
        let mut queued = std::iter::once(first)
            .chain(std::iter::from_fn(|| rx.try_recv().ok()))
            .peekable();
        while let Some(cmd) = queued.next() {
            if matches!(
                (&cmd, queued.peek()),
                (
                    Request::Apply(Action::Volume(_)),
                    Some(Request::Apply(Action::Volume(_)))
                ) | (
                    Request::Apply(Action::Brightness(_)),
                    Some(Request::Apply(Action::Brightness(_)))
                )
            ) {
                continue;
            }

            match cmd {
                Request::Visible(v) => {
                    active = v;
                    dirty = v;
                    if active && audio.is_none() {
                        audio = audio_monitor(tx.clone());
                    }
                    if !active {
                        if let Some(mut c) = audio.take() {
                            let _ = c.kill();
                            let _ = c.wait();
                        }
                    }
                }
                Request::MediaKey(key) => {
                    match media_key(&key, bus.as_ref(), &prefs) {
                        Ok((label, level, muted)) => {
                            let _ = events.try_send(Event::Level(label, level, muted));
                        }
                        Err(e) => eprintln!("Media key: {e:#}"),
                    }
                    dirty = true;
                }
                Request::Changed => dirty = true,
                Request::Apply(a) => {
                    if let Err(e) = apply(a, &d, bus.as_ref(), &prefs) {
                        error = Some(e.to_string());
                    }
                    dirty = true;
                }
            }
        }
        if active && dirty {
            let mut next = snapshot(bus.as_ref(), &prefs);
            next.error = error;
            if next != d {
                d = next;
                let _ = events.try_send(Event::Devices(d.clone()));
            }
        }
    }
}
pub fn watch(
    tx: Sender<Request>,
) -> (
    Vec<gio::SignalSubscription>,
    gio::Settings,
    Vec<gio::FileMonitor>,
) {
    let mut subs = Vec::new();
    if let Ok(bus) = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) {
        for dest in [
            "org.bluez",
            "org.freedesktop.UPower",
            "org.freedesktop.NetworkManager",
        ] {
            let owner = tx.clone();
            subs.push(bus.subscribe_to_signal(
                Some("org.freedesktop.DBus"),
                Some("org.freedesktop.DBus"),
                Some("NameOwnerChanged"),
                Some("/org/freedesktop/DBus"),
                Some(dest),
                gio::DBusSignalFlags::NONE,
                move |_| {
                    let _ = owner.try_send(Request::Changed);
                },
            ));
            let sender = tx.clone();
            subs.push(bus.subscribe_to_signal(
                Some(dest),
                None,
                None,
                None,
                None,
                gio::DBusSignalFlags::NONE,
                move |_| {
                    let _ = sender.try_send(Request::Changed);
                },
            ));
        }
    }
    let mut monitors = Vec::new();
    let root = Path::new("/sys/class/backlight");
    for path in std::iter::once(root.to_owned()).chain(
        fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("brightness")),
    ) {
        if let Ok(m) =
            gio::File::for_path(path).monitor(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let s = tx.clone();
            m.connect_changed(move |_, _, _, _| {
                let _ = s.try_send(Request::Changed);
            });
            monitors.push(m);
        }
    }
    let settings = gio::Settings::new("org.gnome.desktop.notifications");
    settings.connect_changed(Some("show-banners"), move |_, _| {
        let _ = tx.try_send(Request::Changed);
    });
    (subs, settings, monitors)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn laptop_fixture() {
        let root = std::env::temp_dir().join(format!("mundane-backlight-{}", std::process::id()));
        let panel = root.join("virt-panel");
        fs::create_dir_all(&panel).unwrap();
        fs::write(panel.join("max_brightness"), "400").unwrap();
        fs::write(panel.join("brightness"), "100").unwrap();
        let b = read_backlight(&root).unwrap();
        assert_eq!((b.percent, b.units(75)), (25, 300));
        fs::remove_dir_all(root.clone()).unwrap();
        assert_eq!(read_backlight(&root), None);
    }
}
