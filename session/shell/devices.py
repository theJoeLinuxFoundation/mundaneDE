"""Query the distribution's service clients as the session user."""

import os
import re
import subprocess


def command(*argv):
    return subprocess.run(
        argv,
        text=True,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        check=True,
        timeout=20,
        env={**os.environ, "LC_ALL": "C"},
    ).stdout.strip()


def optional(*argv):
    try:
        return command(*argv)
    except (OSError, subprocess.SubprocessError):
        return ""


def bluetooth_devices():
    from gi.repository import Gio, GLib

    try:
        bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        objects = bus.call_sync(
            "org.bluez",
            "/",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None,
            None,
            Gio.DBusCallFlags.NONE,
            1500,
            None,
        ).unpack()[0]
    except GLib.Error:
        return [], False
    devices = [
        i["org.bluez.Device1"] for i in objects.values() if "org.bluez.Device1" in i
    ]
    return [(d["Address"], d.get("Alias") or d["Address"]) for d in devices], any(
        "org.bluez.Adapter1" in i for i in objects.values()
    )


def devices():
    audio = optional("wpctl", "status", "-n")
    sinks = []
    section = False
    for line in audio.splitlines():
        if "Sinks:" in line:
            section = True
        elif re.search(r"(Sources|Filters|Streams|Devices):", line):
            section = False
        elif section:
            match = re.search(r"(\*)?\s*(\d+)\.\s+(.+?)(?:\s+\[vol:.*)?$", line)
            if match:
                sinks.append((match[2], match[3], bool(match[1])))
    battery = battery_status()
    bluetooth, adapter = bluetooth_devices()
    return dict(
        wifi=optional("nmcli", "radio", "wifi"),
        networks=optional(
            "nmcli", "-t", "--escape", "no", "-f", "NAME", "connection", "show"
        ).splitlines(),
        sinks=sinks,
        volume=optional("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"),
        bluetooth=bluetooth,
        adapter=adapter,
        battery=battery,
    )


def battery_status():
    from gi.repository import Gio, GLib

    try:
        bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        props = bus.call_sync(
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            GLib.Variant("(s)", ("org.freedesktop.UPower.Device",)),
            None,
            Gio.DBusCallFlags.NONE,
            1500,
            None,
        ).unpack()[0]
        return (
            f"Battery {props['Percentage']:.0f}%"
            if props.get("IsPresent")
            else "No battery"
        )
    except (GLib.Error, KeyError):
        return "Battery status unavailable"
