//! Feed physical evdev keys to the private headless host using its standard US keymap.
use std::{
    env,
    fs::File,
    os::{fd::AsFd, unix::net::UnixStream},
};
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
delegate_noop!(Driver: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(Driver: ignore ZwlrVirtualPointerV1);
struct Driver;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Driver {
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
delegate_noop!(Driver: ignore wl_seat::WlSeat);
delegate_noop!(Driver: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Driver: ignore ZwpVirtualKeyboardV1);

fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let socket = UnixStream::connect(args.next().expect("private host socket"))?;
    let mode = args.next().expect("keymap path or pointer");
    let conn = Connection::from_socket(socket)?;
    let (globals, mut queue) = registry_queue_init::<Driver>(&conn)?;
    let handle = queue.handle();
    if mode == "pointer" {
        let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&handle, 1..=2, ())?;
        let pointer = manager.create_virtual_pointer(None, &handle, ());
        queue.roundtrip(&mut Driver)?;
        for (i, arg) in args.enumerate() {
            let time = i as u32 * 100;
            if let Some((x, y)) = arg.split_once(',') {
                pointer.motion_absolute(time, x.parse()?, y.parse()?, 800, 450);
            } else {
                let button: i32 = arg.parse()?;
                let state = if button > 0 {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                };
                pointer.button(time, button.unsigned_abs(), state);
            }
            pointer.frame();
            queue.roundtrip(&mut Driver)?;
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return Ok(());
    }
    let keymap = File::open(mode)?;
    let seat = globals.bind(&handle, 1..=1, ())?;
    let manager: ZwpVirtualKeyboardManagerV1 = globals.bind(&handle, 1..=1, ())?;
    let keyboard = manager.create_virtual_keyboard(&seat, &handle, ());
    keyboard.keymap(1, keymap.as_fd(), keymap.metadata()?.len() as u32);
    queue.roundtrip(&mut Driver)?;
    // Give the host a complete first key cycle as it publishes the new seat capability.
    keyboard.key(0, 42, 1);
    queue.roundtrip(&mut Driver)?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    keyboard.key(1, 42, 0);
    queue.roundtrip(&mut Driver)?;
    for (i, arg) in args.enumerate() {
        let code: i32 = arg.parse()?;
        keyboard.key(i as u32 * 30, code.unsigned_abs(), (code > 0) as u32);
        queue.roundtrip(&mut Driver)?;
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    keyboard.destroy();
    queue.roundtrip(&mut Driver)?;
    Ok(())
}
