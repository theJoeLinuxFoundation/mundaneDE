//! Candidate GPUI shell. The compositor supplies both the Wayland connection and control socket.
mod appearance;
mod auth;
mod auth_dialog;
mod bluetooth;
mod cast;
mod catalog;
mod choices;
mod devices;
mod feedback;
mod launcher;
mod notifications;
mod portals;
mod quick;
mod reader;
mod tray;
mod upgrades;
mod view;

use async_channel::{Receiver, Sender};
use catalog::{Application, Favourites};
use gio::{glib, prelude::*};
use gpui::{App, DisplayChanges, DisplayEvent, DisplayId, Global, QuitMode, WindowHandle};
use serde::Deserialize;
use std::{
    collections::HashMap,
    os::{fd::AsFd, unix::net::UnixDatagram},
    sync::Arc,
};
use view::{Page, Surface};

#[derive(Clone, Deserialize, Debug, PartialEq)]
struct Task {
    id: String,
    app: String,
    title: String,
    workspace: u8,
    minimized: bool,
    #[serde(default)]
    active: bool,
}

#[derive(Clone, Default, Deserialize)]
struct Message {
    open: Option<String>,
    osd: Option<String>,
    screen_reader: Option<String>,
    outputs: Option<Vec<String>>,
    #[serde(default)]
    dismiss_menu: bool,
    #[serde(default)]
    escape_menu: bool,
    #[serde(default)]
    logout: bool,
    locked: Option<bool>,
    workspace: Option<u8>,
    #[serde(default)]
    tasks: Vec<Task>,
    #[serde(default)]
    selected: usize,
    #[serde(default)]
    start: usize,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    pointer: Option<[f32; 2]>,
}
#[derive(Clone)]
enum Service {
    Identity(u64, u32),
    Answer(u64, u64, zeroize::Zeroizing<String>),
    CancelAuth(u64),
    ReaderToggle,
    ReaderKey(reader::Key),
    Logout,
    NoteAction(u32, Option<String>),
    LocalNotice(String, String),
    Accept(u64, String, String, String),
    Cancel(u64),
    Stop(String),
    Locked(bool),
    Pair(String, String),
    PairAnswer(u64, String),
    CancelPair,
    Launch(String),
    Pin(String),
    TrayActivate(String),
    Menu(String, i32),
    Select(String, i32),
}
#[derive(Clone)]
enum Event {
    Auth(auth::Prompt),
    AuthClosed(u64),
    Banner(notifications::Note, i32),
    History(Vec<notifications::Note>),
    HideBanner(Option<u32>),
    Level(&'static str, u8, bool),
    UpgradeDetails(Vec<String>),
    Picker(portals::Picker),
    PickerClosed(u64),
    Shares(Vec<(String, String)>),
    Devices(devices::Devices),
    PairPrompt(Option<bluetooth::Prompt>),
    DeviceMessage(String),
    Message(Message),
    Switcher(Message, Vec<String>),
    HideSwitcher,
    Catalog(Vec<Application>),
    Pins(Vec<String>),
    Tray(Vec<tray::Item>),
    TrayChanged(String),
    MenuRows(String, i32, Vec<tray::Entry>),
    Clock(String),
    ReduceMotion(bool),
    TextScale(f32),
    Open(DisplayId, Page),
    OpenAt(DisplayId, Page, f32),
    Dismiss,
    Command(String),
    Service(Service),
    Quit,
}
struct Shell {
    auth: Option<auth::Prompt>,
    retired_auth: u64,
    locked: bool,
    outputs: Vec<String>,
    picker: Option<portals::Picker>,
    shares: Vec<(String, String)>,
    notes: Vec<notifications::Note>,
    upgrades: Vec<String>,
    overlays: feedback::Overlays,
    devices: devices::Devices,
    device_tx: Sender<devices::Request>,
    pairing: Option<bluetooth::Prompt>,
    device_message: String,
    events: Sender<Event>,
    service: Sender<Service>,
    socket: Arc<UnixDatagram>,
    apps: Vec<Application>,
    pins: Vec<String>,
    tasks: Vec<Task>,
    desk: u8,
    tray: Vec<tray::Item>,
    clock: String,
    text_scale: f32,
    bars: HashMap<DisplayId, WindowHandle<Surface>>,
    menu: Option<(DisplayId, Page, WindowHandle<Surface>)>,
    rows: Vec<tray::Entry>,
    switcher: Vec<String>,
    selected: usize,
    position: usize,
    count: usize,
}
impl Global for Shell {}

fn services(
    socket: Arc<UnixDatagram>,
    tx: Sender<Event>,
    rx: Receiver<Service>,
    ready: Receiver<Sender<devices::Request>>,
) -> anyhow::Result<()> {
    let context = glib::MainContext::new();
    context.with_thread_default(|| -> anyhow::Result<()> {
        let pins = std::rc::Rc::new(Favourites::new());
        let publish = {
            let tx = tx.clone();
            let pins = pins.clone();
            move || {
                let _ = tx.try_send(Event::Pins(pins.get()));
            }
        };
        publish();
        pins.watch(publish);
        let reload = {
            let tx = tx.clone();
            move || {
                let _ = tx.try_send(Event::Catalog(catalog::installed()));
            }
        };
        let monitor = gio::AppInfoMonitor::get();
        monitor.connect_changed({
            let reload = reload.clone();
            move |_| reload()
        });
        reload();
        // Catalogue discovery overlaps GPUI platform startup. Start services that
        // launch clients only after GPUI has consumed the private Wayland fd.
        let device_tx = ready.recv_blocking()?;
        let reader = reader::Reader::new(socket.clone())?;
        let auth = auth::Agent::new(reader.clone(), tx.clone())
            .map_err(|e| eprintln!("Polkit agent: {e:#}"))
            .ok();
        let notes = notifications::Server::new(tx.clone())?;
        let portal = portals::Portals::new(tx.clone(), socket)?;
        let recovery = portal.clone();
        let _appearance = appearance::register(move || recovery.recover_closed())?;
        let _devices = devices::watch(device_tx);
        let agent = bluetooth::Agent::new(tx.clone()).ok();
        let tray = tray::Tray::new(tx.clone())?;
        let desktop = gio::Settings::new("org.gnome.desktop.interface");
        let motion = tx.clone();
        let report = move |settings: &gio::Settings| {
            let _ = motion.try_send(Event::ReduceMotion(!settings.boolean("enable-animations")));
        };
        report(&desktop);
        desktop.connect_changed(Some("enable-animations"), move |settings, _| {
            report(settings)
        });
        let scale_tx = tx.clone();
        let text_scale = move |settings: &gio::Settings| {
            let factor = settings.double("text-scaling-factor") as f32;
            if factor.is_finite() && factor > 0. {
                let _ = scale_tx.try_send(Event::TextScale(factor));
            }
        };
        text_scale(&desktop);
        desktop.connect_changed(Some("text-scaling-factor"), move |settings, _| {
            text_scale(settings)
        });
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let done = main_loop.clone();
        context.spawn_local(async move {
            while let Ok(action) = rx.recv().await {
                match action {
                    Service::LocalNotice(t, b) => notes.local(t, b),
                    Service::NoteAction(id, key) => notes.action(id, key),
                    Service::Accept(seq, kind, id, label) => portal.accept(seq, &kind, &id, label),
                    Service::Cancel(seq) => portal.cancel(seq),
                    Service::Stop(path) => portal.close(&path),
                    Service::Locked(v) => {
                        portal.locked(v);
                        if let Some(a) = &auth {
                            a.lock(v)
                        }
                    }
                    Service::Identity(id, uid) => {
                        if let Some(a) = &auth {
                            a.select(id, uid)
                        }
                    }
                    Service::Answer(id, seq, value) => {
                        if let Some(a) = &auth {
                            a.respond(id, seq, &value)
                        }
                    }
                    Service::CancelAuth(id) => {
                        if let Some(a) = &auth {
                            a.cancel(id)
                        }
                    }
                    Service::ReaderToggle => reader.toggle(),
                    Service::ReaderKey(key) => reader.key(key),
                    Service::Logout => {
                        if let Some(a) = &auth {
                            a.shutdown()
                        }
                        let _ = tx.try_send(Event::Quit);
                        break;
                    }
                    Service::Pair(path, name) => {
                        let result = agent
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("System bus unavailable"))
                            .and_then(|a| a.pair(path, name));
                        if let Err(e) = result {
                            let _ = tx.try_send(Event::DeviceMessage(e.to_string()));
                        }
                    }
                    Service::CancelPair => {
                        if let Some(a) = &agent {
                            a.cancel();
                        }
                    }
                    Service::PairAnswer(id, input) => {
                        if let Some(a) = &agent {
                            a.answer(id, input);
                        }
                    }
                    Service::Launch(id) => {
                        if let Some(app) = gio_unix::DesktopAppInfo::new(&id) {
                            if let Err(e) = app.launch(&[], None::<&gio::AppLaunchContext>) {
                                eprintln!("Launch {id}: {e}")
                            }
                        }
                    }
                    Service::Pin(id) => {
                        if let Err(e) = pins.toggle(&id) {
                            eprintln!("Save favourites: {e}")
                        }
                    }
                    Service::TrayActivate(key) => tray.activate(&key),
                    Service::Menu(key, parent) => {
                        let rows = tray.menu(&key, parent).unwrap_or_else(|e| {
                            eprintln!("Tray menu: {e}");
                            Vec::new()
                        });
                        let _ = tx.try_send(Event::MenuRows(key, parent, rows));
                    }
                    Service::Select(key, id) => tray.select(&key, id),
                }
            }
            done.quit();
        });
        main_loop.run();
        Ok(())
    })?
}

fn close_menu(cx: &mut App) {
    if let Some((_, page, window)) = cx.global_mut::<Shell>().menu.take() {
        if let Page::Auth(id) = page {
            let s = cx.global_mut::<Shell>();
            s.retired_auth = id;
            if s.auth.take().is_some() {
                let _ = s.service.try_send(Service::CancelAuth(id));
            }
        }
        if let Page::Capture(id) = page {
            let s = cx.global_mut::<Shell>();
            if s.picker.as_ref().is_some_and(|p| p.serial == id) {
                s.picker = None;
                let _ = s.service.try_send(Service::Cancel(id));
            }
        }
        if page == Page::System {
            let s = cx.global_mut::<Shell>();
            let _ = s.device_tx.try_send(devices::Request::Visible(false));
            let _ = s.service.try_send(Service::CancelPair);
            s.pairing = None;
            s.device_message.clear();
        }
        let _ = window.update(cx, |_, w, _| w.remove_window());
        cx.refresh_windows();
    }
}
fn open_menu(display: DisplayId, page: Page, anchor: Option<f32>, cx: &mut App) {
    let same = cx
        .global::<Shell>()
        .menu
        .as_ref()
        .is_some_and(|(d, p, _)| *d == display && *p == page);
    close_menu(cx);
    if same {
        return;
    }
    if let Page::Tray(key, stack) = &page {
        let state = cx.global_mut::<Shell>();
        state.rows.clear();
        let _ = state
            .service
            .try_send(Service::Menu(key.clone(), *stack.last().unwrap_or(&0)));
    }
    match view::open(display, page.clone(), anchor, cx) {
        Ok(w) => {
            if page == Page::System {
                let _ = cx
                    .global::<Shell>()
                    .device_tx
                    .try_send(devices::Request::Visible(true));
            }
            cx.global_mut::<Shell>().menu = Some((display, page, w));
            cx.refresh_windows();
        }
        Err(e) => eprintln!("Open menu: {e:#}"),
    }
}

fn display_bars(cx: &mut App) {
    for display in cx.displays() {
        if !cx.global::<Shell>().bars.contains_key(&display.id()) {
            match view::open(display.id(), Page::Bar, None, cx) {
                Ok(window) => {
                    cx.global_mut::<Shell>().bars.insert(display.id(), window);
                }
                Err(e) => {
                    eprintln!("Taskbar on {:?}: {e:#}", display.id());
                    cx.quit();
                }
            }
        }
    }
}

// Wayland has no standard primary-output designation. The compositor positions
// its primary at the desktop origin; fall back to the first live display.
fn home_display(cx: &App) -> Option<std::rc::Rc<dyn gpui::PlatformDisplay>> {
    let displays = cx.displays();
    displays
        .iter()
        .find(|d| {
            d.bounds()
                .contains(&gpui::point(gpui::px(0.), gpui::px(0.)))
        })
        .cloned()
        .or_else(|| displays.into_iter().next())
}
fn target(m: &Message, cx: &App) -> Option<DisplayId> {
    m.pointer
        .and_then(|[x, y]| {
            cx.displays()
                .into_iter()
                .find(|d| d.bounds().contains(&gpui::point(gpui::px(x), gpui::px(y))))
                .map(|d| d.id())
        })
        .or_else(|| home_display(cx).map(|d| d.id()))
}
fn handle(event: Event, cx: &mut App) {
    match event {
        Event::Auth(p) => {
            let id = p.id;
            if id <= cx.global::<Shell>().retired_auth {
                return;
            }
            if cx.global::<Shell>().locked || home_display(cx).is_none() {
                let _ = cx
                    .global::<Shell>()
                    .service
                    .try_send(Service::CancelAuth(id));
                return;
            }
            let already = cx
                .global::<Shell>()
                .menu
                .as_ref()
                .is_some_and(|m| m.1 == Page::Auth(id));
            if !already {
                close_menu(cx)
            }
            cx.global_mut::<Shell>().auth = Some(p);
            if !already {
                open_menu(home_display(cx).unwrap().id(), Page::Auth(id), None, cx);
                if cx.global::<Shell>().menu.is_none() {
                    let _ = cx
                        .global::<Shell>()
                        .service
                        .try_send(Service::CancelAuth(id));
                }
            }
        }
        Event::AuthClosed(id) => {
            if cx
                .global::<Shell>()
                .auth
                .as_ref()
                .is_some_and(|a| a.id == id)
            {
                cx.global_mut::<Shell>().auth = None;
                close_menu(cx)
            }
            return;
        }
        Event::History(v) => cx.global_mut::<Shell>().notes = v,
        Event::Banner(n, ms) => {
            feedback::show(feedback::Feedback::Banner(n), ms, cx);
            return;
        }
        Event::HideBanner(id) => {
            feedback::hide_banner(id, cx);
            return;
        }
        Event::Level(label, level, muted) => {
            feedback::show(feedback::Feedback::Level(label, level, muted), 1500, cx);
            return;
        }
        Event::UpgradeDetails(v) => cx.global_mut::<Shell>().upgrades = v,
        Event::Shares(v) => cx.global_mut::<Shell>().shares = v,
        Event::Picker(p) => {
            let id = p.serial;
            if let Some(d) = home_display(cx).filter(|_| !cx.global::<Shell>().locked) {
                close_menu(cx);
                cx.global_mut::<Shell>().picker = Some(p);
                let _ = cx.global::<Shell>().socket.send(b"list");
                open_menu(d.id(), Page::Capture(id), None, cx);
                if cx.global::<Shell>().menu.is_some() {
                    return;
                }
            }
            cx.global_mut::<Shell>().picker = None;
            let _ = cx.global::<Shell>().service.try_send(Service::Cancel(id));
            return;
        }
        Event::PickerClosed(id) => {
            if cx
                .global::<Shell>()
                .picker
                .as_ref()
                .is_some_and(|p| p.serial == id)
            {
                cx.global_mut::<Shell>().picker = None;
                close_menu(cx);
            }
            return;
        }
        Event::Service(Service::Accept(id, kind, source, label)) => {
            if cx
                .global::<Shell>()
                .picker
                .as_ref()
                .is_some_and(|p| p.serial == id)
            {
                cx.global_mut::<Shell>().picker = None;
                close_menu(cx);
                let _ = cx
                    .global::<Shell>()
                    .service
                    .try_send(Service::Accept(id, kind, source, label));
            }
            return;
        }
        Event::Devices(d) => cx.global_mut::<Shell>().devices = d,
        Event::PairPrompt(p) => {
            if !matches!(cx.global::<Shell>().menu, Some((_, Page::System, _))) {
                return;
            }
            cx.global_mut::<Shell>().pairing = p;
        }
        Event::DeviceMessage(m) => cx.global_mut::<Shell>().device_message = m,
        Event::Message(mut m) => {
            if m.logout {
                let _ = cx.global::<Shell>().service.try_send(Service::Logout);
                return;
            }
            if let Some(locked) = m.locked {
                cx.global_mut::<Shell>().locked = locked;
                let _ = cx
                    .global::<Shell>()
                    .service
                    .try_send(Service::Locked(locked));
                if locked {
                    feedback::hide_banner(None, cx);
                    feedback::hide_level(cx);
                }
            }
            if m.screen_reader.as_deref() == Some("toggle") {
                let _ = cx.global::<Shell>().service.try_send(Service::ReaderToggle);
            }
            if let Some(delta) = m.osd.take() {
                let _ = cx
                    .global::<Shell>()
                    .device_tx
                    .try_send(devices::Request::MediaKey(delta));
            }
            if m.escape_menu {
                let window = cx.global::<Shell>().menu.as_ref().map(|(_, _, w)| *w);
                if let Some(window) = window {
                    if !window
                        .update(cx, |v, w, cx| v.escape(w, cx))
                        .unwrap_or(false)
                    {
                        close_menu(cx);
                    }
                }
            }
            if m.dismiss_menu || m.locked == Some(true) {
                close_menu(cx);
            }
            if let Some(d) = target(&m, cx) {
                match m.open.as_deref() {
                    Some("start") => open_menu(d, Page::Start, None, cx),
                    Some("taskbar") => {
                        let shell = cx.global::<Shell>();
                        if let Some(bar) = shell.bars.get(&d).copied() {
                            let _ = shell.socket.send(b"focus-taskbar");
                            let _ = bar.update(cx, |v, w, cx| v.focus_start(w, cx));
                        }
                    }
                    Some("system") => open_menu(d, Page::System, None, cx),
                    _ => {}
                }
            }
            if let Some(desk) = m.workspace {
                m.tasks.sort_by(|a, b| {
                    (&a.workspace, &a.title, &a.id).cmp(&(&b.workspace, &b.title, &b.id))
                });
                let s = cx.global_mut::<Shell>();
                let outputs = m.outputs.take().unwrap_or_default();
                if s.desk == desk && s.tasks == m.tasks && s.outputs == outputs {
                    return;
                }
                s.outputs = outputs;
                s.desk = desk;
                s.tasks = m.tasks;
            } else {
                return;
            }
        }
        Event::Switcher(m, titles) => {
            let d = target(&m, cx);
            let s = cx.global_mut::<Shell>();
            s.switcher = titles;
            s.selected = m.selected.saturating_sub(m.start);
            s.count = m.count;
            s.position = m.selected + 1;
            if let Some(d) = d {
                if !s
                    .menu
                    .as_ref()
                    .is_some_and(|(id, p, _)| *id == d && *p == Page::Switcher)
                {
                    open_menu(d, Page::Switcher, None, cx);
                }
            }
        }
        Event::HideSwitcher => {
            if matches!(cx.global::<Shell>().menu, Some((_, Page::Switcher, _))) {
                close_menu(cx);
            }
            return;
        }
        Event::Open(d, p) => {
            open_menu(d, p, None, cx);
            return;
        }
        Event::OpenAt(d, p, x) => {
            open_menu(d, p, Some(x), cx);
            return;
        }
        Event::Dismiss => {
            close_menu(cx);
            return;
        }
        Event::Quit => {
            cx.quit();
            return;
        }
        Event::Command(cmd) => {
            close_menu(cx);
            let _ = cx.global::<Shell>().socket.send(cmd.as_bytes());
            return;
        }
        Event::Service(action) => {
            if !matches!(action, Service::Pin(_)) {
                close_menu(cx);
            }
            let _ = cx.global::<Shell>().service.try_send(action);
            return;
        }
        Event::Pins(v) => cx.global_mut::<Shell>().pins = v,
        Event::Catalog(v) => cx.global_mut::<Shell>().apps = v,
        Event::TextScale(factor) => {
            let s = cx.global_mut::<Shell>();
            if s.text_scale == factor {
                return;
            }
            s.text_scale = factor;
            let bars = std::mem::take(&mut s.bars);
            for (_, bar) in bars {
                let _ = bar.update(cx, |_, w, _| w.remove_window());
            }
            display_bars(cx);
        }
        Event::ReduceMotion(reduce) => {
            let cmd = if reduce {
                "reduced-motion on"
            } else {
                "reduced-motion off"
            };
            let _ = cx.global::<Shell>().socket.send(cmd.as_bytes());
            return;
        }
        Event::Clock(v) => cx.global_mut::<Shell>().clock = v,
        Event::Tray(v) => {
            let s = cx.global_mut::<Shell>();
            if s.tray == v {
                return;
            }
            s.tray = v;
            if let Some((_, Page::Tray(k, _), _)) = &s.menu {
                if !s.tray.iter().any(|i| &i.key == k) {
                    close_menu(cx);
                }
            }
        }
        Event::TrayChanged(k) => {
            let s = cx.global::<Shell>();
            if let Some((_, Page::Tray(key, parents), _)) = &s.menu {
                if key == &k {
                    let _ = s
                        .service
                        .try_send(Service::Menu(k, *parents.last().unwrap()));
                }
            }
            return;
        }
        Event::MenuRows(k, parent, rows) => {
            let s = cx.global_mut::<Shell>();
            if matches!(&s.menu,Some((_,Page::Tray(key,stack),_)) if key==&k && stack.last()==Some(&parent))
            {
                s.rows = rows;
            } else {
                return;
            }
        }
    }
    cx.refresh_windows();
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [flag, kind, id, cursor] = args.as_slice() {
        if flag == "--cast-worker" {
            return cast::worker(kind, id, cursor);
        }
    }
    anyhow::ensure!(
        std::env::var("WAYLAND_SOCKET").as_deref() == Ok("0"),
        "Start mundane-shell through mundane-session --gpui"
    );
    // Own a CLOEXEC duplicate. Keep logging and descendants away from the shell command channel.
    let fd = std::io::stdout().as_fd().try_clone_to_owned()?;
    let socket = Arc::new(UnixDatagram::from(fd));
    socket.peer_addr()?;
    anyhow::ensure!(unsafe { libc::dup2(2, 1) } == 1, "redirect shell stdout");
    let (tx, rx) = async_channel::unbounded();
    let (service, actions) = async_channel::unbounded();
    let (ready, platform) = async_channel::bounded(1);
    let service_socket = socket.clone();
    let worker = tx.clone();
    std::thread::spawn(move || {
        if let Err(e) = services(service_socket, worker.clone(), actions, platform) {
            eprintln!("Shell services: {e:#}");
            let _ = worker.try_send(Event::Quit);
        }
    });
    mundane_ui::run(move |cx| {
        let input = socket.clone();
        let messages = tx.clone();
        let keys = service.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0; 262144];
            while let Ok(n) = input.recv(&mut buf) {
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&buf[..n]) else {
                    continue;
                };
                if json.get("subscription").is_some() {
                    if let Ok(key) = serde_json::from_value(json) {
                        let _ = keys.try_send(Service::ReaderKey(key));
                    }
                    continue;
                }
                let Ok(m) = serde_json::from_value::<Message>(json.clone()) else {
                    continue;
                };
                let event = match json.get("switcher") {
                    Some(v) if v.is_null() => Event::HideSwitcher,
                    Some(v) => {
                        Event::Switcher(m, serde_json::from_value(v.clone()).unwrap_or_default())
                    }
                    None => Event::Message(m),
                };
                if messages.send_blocking(event).is_err() {
                    return;
                }
            }
            let _ = messages.try_send(Event::Quit);
        });
        let clock = tx.clone();
        std::thread::spawn(move || loop {
            let Ok(now) = glib::DateTime::now_local() else {
                return;
            };
            if clock
                .send_blocking(Event::Clock(
                    now.format("%H:%M").unwrap_or_default().to_string(),
                ))
                .is_err()
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(60 - now.second() as u64));
        });
        mundane_ui::bind_text_keys(cx);
        cx.set_quit_mode(QuitMode::Explicit);
        let device_tx = devices::start(tx.clone());
        let _ = ready.try_send(device_tx.clone());
        upgrades::start(tx.clone(), service.clone());
        let _ = socket.send(b"watch-tasks");
        cx.set_global(Shell {
            auth: None,
            retired_auth: 0,
            locked: true,
            outputs: vec![],
            picker: None,
            shares: vec![],
            notes: vec![],
            upgrades: vec![],
            overlays: Default::default(),
            devices: Default::default(),
            device_tx,
            pairing: None,
            device_message: String::new(),
            events: tx,
            service,
            socket,
            apps: vec![],
            pins: vec![],
            tasks: vec![],
            desk: 1,
            tray: vec![],
            clock: String::new(),
            text_scale: 1.,
            bars: HashMap::new(),
            menu: None,
            rows: vec![],
            switcher: vec![],
            selected: 0,
            position: 0,
            count: 0,
        });
        display_bars(cx);
        cx.observe_displays(|ev, cx| {
            let _ = cx.global::<Shell>().socket.send(b"list");
            if matches!(ev, DisplayEvent::Removed(_)) {
                feedback::hide_banner(None, cx);
                feedback::hide_level(cx);
            }
            if let DisplayEvent::Removed(d) = ev {
                if cx
                    .global::<Shell>()
                    .menu
                    .as_ref()
                    .is_some_and(|(id, _, _)| *id == d)
                {
                    close_menu(cx);
                }
                if let Some(w) = cx.global_mut::<Shell>().bars.remove(&d) {
                    let _ = w.update(cx, |_, w, _| w.remove_window());
                }
            }
            if let DisplayEvent::Changed { id, changes } = ev {
                if changes.contains(DisplayChanges::BOUNDS) {
                    if cx
                        .global::<Shell>()
                        .menu
                        .as_ref()
                        .is_some_and(|(d, _, _)| *d == id)
                    {
                        close_menu(cx);
                    }
                    if let (Some(w), Some(d)) = (
                        cx.global::<Shell>().bars.get(&id).copied(),
                        cx.displays().into_iter().find(|d| d.id() == id),
                    ) {
                        let height = gpui::px(44. * cx.global::<Shell>().text_scale);
                        let _ = w.update(cx, |_, win, _| {
                            win.resize(gpui::size(d.bounds().size.width, height))
                        });
                    }
                }
            }
            display_bars(cx);
        })
        .detach();
        cx.spawn(async move |cx| {
            while let Ok(event) = rx.recv().await {
                cx.update(|cx| handle(event, cx));
            }
        })
        .detach();
    });
    Ok(())
}
