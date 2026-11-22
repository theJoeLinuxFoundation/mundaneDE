//! Stream ext-foreign-toplevel-list updates to the GTK panel's ordinary client connection.
use serde::Serialize;
use std::{collections::HashMap, io::Read};
use wayland_client::{
    backend::ObjectId,
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1 as handle, ext_foreign_toplevel_list_v1 as list,
};

#[derive(Default, Serialize)]
struct Entry {
    id: String,
    title: String,
    app: String,
}
#[derive(Default)]
struct Watch {
    entries: HashMap<ObjectId, Entry>,
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Watch {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<list::ExtForeignToplevelListV1, ()> for Watch {
    fn event(
        _: &mut Self,
        _: &list::ExtForeignToplevelListV1,
        _: list::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    wayland_client::event_created_child!(Watch,list::ExtForeignToplevelListV1,[0=>(handle::ExtForeignToplevelHandleV1,())]);
}
impl Dispatch<handle::ExtForeignToplevelHandleV1, ()> for Watch {
    fn event(
        s: &mut Self,
        h: &handle::ExtForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let entry = s.entries.entry(h.id()).or_default();
        match event {
            handle::Event::Identifier { identifier } => entry.id = identifier,
            handle::Event::Title { title } => entry.title = title,
            handle::Event::AppId { app_id } => entry.app = app_id,
            handle::Event::Done => println!("{}", serde_json::to_string(entry).unwrap()),
            handle::Event::Closed => {
                println!("{}", serde_json::json!({"closed":entry.id}));
                s.entries.remove(&h.id());
                h.destroy();
            }
            _ => {}
        }
    }
}
pub fn watch() -> anyhow::Result<()> {
    // The panel holds the write end. Exit even if it dies with no windows open.
    std::thread::spawn(|| {
        let _ = std::io::stdin().read(&mut [0u8]);
        std::process::exit(0);
    });
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Watch>(&conn)?;
    let _list: list::ExtForeignToplevelListV1 = globals.bind(&queue.handle(), 1..=1, ())?;
    let mut state = Watch::default();
    loop {
        queue.blocking_dispatch(&mut state)?;
    }
}
