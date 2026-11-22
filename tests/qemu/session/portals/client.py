"""An ordinary guest application using the public desktop portal."""

import os
import subprocess
import sys
import uuid
from pathlib import Path

from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
root = "/org/freedesktop/portal/desktop"
name = "org.freedesktop.portal.Desktop"
loop = GLib.MainLoop()
base = Path(os.environ["XDG_RUNTIME_DIR"]) / "mundane-portal-test"
base.mkdir(exist_ok=True)


def call(iface, method, signature, args):
    return bus.call_sync(
        name,
        root,
        "org.freedesktop.portal." + iface,
        method,
        GLib.Variant(signature, args),
        None,
        0,
        15000,
        None,
    ).unpack()


def request(iface, method, prefix, values, opts):
    token = "r" + uuid.uuid4().hex
    path = (
        root + "/request/" + bus.get_unique_name()[1:].replace(".", "_") + "/" + token
    )
    results = []

    def got(b, s, p, i, m, args):
        results.append(args.unpack())
        loop.quit()

    sid = bus.signal_subscribe(
        name, "org.freedesktop.portal.Request", "Response", path, None, 0, got
    )
    assert call(
        iface,
        method,
        "(" + prefix + "a{sv})",
        (*values, dict(opts, handle_token=GLib.Variant("s", token))),
    ) == (path,)
    loop.run()
    bus.signal_unsubscribe(sid)
    print(method, results[0], flush=True)
    return results[0]


bus.call_sync(
    name,
    root,
    "org.freedesktop.host.portal.Registry",
    "Register",
    GLib.Variant("(sa{sv})", ("org.joelinux.PortalTest", {})),
    None,
    0,
    5000,
    None,
)
mode = sys.argv[1]
if mode == "screenshot":
    code, doc = request(
        "Screenshot", "Screenshot", "s", ("",), {"interactive": GLib.Variant("b", True)}
    )
    if not code:
        import gi

        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf

        path = Gio.File.new_for_uri(doc["uri"]).get_path()
        px = GdkPixbuf.Pixbuf.new_from_file(path)
        w, h = px.get_width(), px.get_height()
        data = px.get_pixels()

        def color(x, y):
            n = int(h * y) * px.get_rowstride() + int(w * x) * px.get_n_channels()
            return data[n : n + 3].hex()

        print("IMAGE", w, h, color(0.5, 0.25), color(0.5, 0.75), flush=True)
elif mode == "cast":
    # A second consumer uses only the public frontend's restricted PipeWire fd.
    code, info = request(
        "ScreenCast",
        "CreateSession",
        "",
        (),
        {"session_handle_token": GLib.Variant("s", "s" + uuid.uuid4().hex)},
    )
    assert code == 0
    session = info["session_handle"]
    assert (
        request(
            "ScreenCast",
            "SelectSources",
            "o",
            (session,),
            {"types": GLib.Variant("u", 2)},
        )[0]
        == 0
    )
    code, info = request("ScreenCast", "Start", "os", (session, ""), {})
    assert code == 0
    [(node, metadata)] = info["streams"]
    assert metadata["source_type"] == 2 and metadata["pipewire-serial"] > 0
    result, fds = bus.call_with_unix_fd_list_sync(
        name,
        root,
        "org.freedesktop.portal.ScreenCast",
        "OpenPipeWireRemote",
        GLib.Variant("(oa{sv})", (session, {})),
        None,
        0,
        5000,
        None,
        None,
    )
    fd = fds.get(result.unpack()[0])
    with subprocess.Popen(
        [
            "gst-launch-1.0",
            "-q",
            "pipewiresrc",
            # Keep the preroll buffer independent of the first playing frame.
            "min-buffers=3",
            "do-timestamp=true",
            "fd=" + str(fd),
            "path=" + str(node),
            "!",
            "videoconvert",
            "!",
            "videoscale",
            "!",
            "video/x-raw,format=RGB,width=32,height=24",
            "!",
            "fdsink",
            "sync=false",
            "async=false",
            "fd=1",
        ],
        pass_fds=(fd,),
        stdout=subprocess.PIPE,
    ) as consumer:
        try:
            while frame := consumer.stdout.read(32 * 24 * 3):
                print(
                    "WINDOW", frame[624:627].hex(), frame[1776:1779].hex(), flush=True
                )
                if frame[624:627] == bytes.fromhex("cc1919") and frame[
                    1776:1779
                ] == bytes.fromhex("8c1919"):
                    break
        finally:
            consumer.terminate()
            os.close(fd)
    bus.call_sync(
        name,
        session,
        "org.freedesktop.portal.Session",
        "Close",
        None,
        None,
        0,
        5000,
        None,
    )
elif mode == "choose":
    request(
        "FileChooser",
        "OpenFile",
        "ss",
        ("", "Portal test file"),
        {"current_folder": GLib.Variant("ay", os.fsencode(base) + b"\0")},
    )
elif mode == "open":
    request("OpenURI", "OpenURI", "ss", ("", "mundane-portal-test://item/42"), {})
elif mode == "notify":
    call(
        "Notification",
        "AddNotification",
        "(sa{sv})",
        (
            "sample",
            {
                "title": GLib.Variant("s", "Portal notification"),
                "body": GLib.Variant("s", "Guest application"),
                "buttons": GLib.Variant(
                    "aa{sv}",
                    [
                        {
                            "label": GLib.Variant("s", "Review capture"),
                            "action": GLib.Variant("s", "review"),
                        }
                    ],
                ),
            },
        ),
    )
    bus.signal_subscribe(
        name,
        "org.freedesktop.portal.Notification",
        "ActionInvoked",
        root,
        None,
        0,
        lambda *args: (
            print("NOTIFICATION ACTION", args[-1].unpack(), flush=True),
            loop.quit(),
        ),
    )
    print("Notification delivered", flush=True)
    loop.run()
