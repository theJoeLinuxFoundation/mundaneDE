//! Candidate GPUI shell. The compositor supplies both the Wayland connection and control socket.
mod catalog;
mod choices;
mod tray;
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
}

#[derive(Clone, Default, Deserialize)]
struct Message {
    open: Option<String>,
    #[serde(default)]
    dismiss_menu: bool,
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
    Launch(String),
    Pin(String),
    TrayActivate(String),
    Menu(String, i32),
    Select(String, i32),
}
#[derive(Clone)]
enum Event {
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
    Open(DisplayId, Page),
    OpenAt(DisplayId, Page, f32),
    Dismiss,
    Command(String),
    Service(Service),
    Quit,
}
struct Shell {
    events: Sender<Event>,
    service: Sender<Service>,
    socket: Arc<UnixDatagram>,
    apps: Vec<Application>,
    pins: Vec<String>,
    tasks: Vec<Task>,
    desk: u8,
    tray: Vec<tray::Item>,
    clock: String,
    bars: HashMap<DisplayId, WindowHandle<Surface>>,
    menu: Option<(DisplayId, Page, WindowHandle<Surface>)>,
    rows: Vec<tray::Entry>,
    switcher: Vec<String>,
    selected: usize,
    position: usize,
    count: usize,
}
impl Global for Shell {}

fn services(tx: Sender<Event>, rx: Receiver<Service>) -> anyhow::Result<()> {
    let context = glib::MainContext::new();
    context.with_thread_default(|| -> anyhow::Result<()> {
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
        reload();
        let monitor = gio::AppInfoMonitor::get();
        monitor.connect_changed(move |_| reload());
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let done = main_loop.clone();
        context.spawn_local(async move {
            while let Ok(action) = rx.recv().await {
                match action {
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
    if let Some((_, _, window)) = cx.global_mut::<Shell>().menu.take() {
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

fn target(m: &Message, cx: &App) -> Option<DisplayId> {
    m.pointer
        .and_then(|[x, y]| {
            cx.displays()
                .into_iter()
                .find(|d| d.bounds().contains(&gpui::point(gpui::px(x), gpui::px(y))))
                .map(|d| d.id())
        })
        .or_else(|| cx.primary_display().map(|d| d.id()))
}
fn handle(event: Event, cx: &mut App) {
    match event {
        Event::Message(mut m) => {
            if m.logout {
                cx.quit();
                return;
            }
            if m.dismiss_menu || m.locked == Some(true) {
                close_menu(cx);
            }
            if let Some(d) = target(&m, cx) {
                match m.open.as_deref() {
                    Some("start") => open_menu(d, Page::Start, None, cx),
                    Some("system") => open_menu(d, Page::System, None, cx),
                    _ => {}
                }
            }
            if let Some(desk) = m.workspace {
                m.tasks.sort_by(|a, b| {
                    (&a.workspace, &a.title, &a.id).cmp(&(&b.workspace, &b.title, &b.id))
                });
                let s = cx.global_mut::<Shell>();
                if s.desk == desk && s.tasks == m.tasks {
                    return;
                }
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
    mundane_ui::run(move |cx| {
        let input = socket.clone();
        let messages = tx.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0; 262144];
            while let Ok(n) = input.recv(&mut buf) {
                let Ok(json) = serde_json::from_slice::<serde_json::Value>(&buf[..n]) else {
                    continue;
                };
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
        let worker = tx.clone();
        std::thread::spawn(move || {
            if let Err(e) = services(worker.clone(), actions) {
                eprintln!("Shell services: {e:#}");
                let _ = worker.try_send(Event::Quit);
            }
        });
        let _ = socket.send(b"watch-tasks");
        cx.set_global(Shell {
            events: tx,
            service,
            socket,
            apps: vec![],
            pins: vec![],
            tasks: vec![],
            desk: 1,
            tray: vec![],
            clock: String::new(),
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
                        let _ = w.update(cx, |_, win, _| {
                            win.resize(gpui::size(d.bounds().size.width, gpui::px(44.)))
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
