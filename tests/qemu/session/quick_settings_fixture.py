"""Exercise laptop-only controls with a sensor snapshot in the headless guest."""

import sys
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, Gtk

sys.path.insert(0, "/usr/local/share/mundane/shell")
from quick_settings import QuickSettings
from devices import backlight

Gtk.init()
with TemporaryDirectory() as temp:
    root = Path(temp)
    assert backlight(root) is None
    device = root / "virt-panel"
    device.mkdir()
    (device / "max_brightness").write_text("400")
    (device / "brightness").write_text("100")
    calls = []
    model = SimpleNamespace(
        devices=dict(
            backlight=backlight(root), battery="Battery 72%", adapter=True, powered=True
        ),
        notifications=Gio.Settings.new("org.gnome.desktop.notifications"),
        bus=SimpleNamespace(notes={}),
        upgrades=SimpleNamespace(details=[]),
        run_action=lambda *args: calls.append(args),
    )
    quick = QuickSettings(model, Gtk.Box(), 480)
    assert quick.battery.get_visible() and quick.battery.get_text() == "Battery 72%"
    assert (
        quick.brightness.box.get_visible() and quick.brightness.scale.get_value() == 25
    )
    quick.set_brightness(75)
    assert calls[-1][-5:] == ("SetBrightness", "ssu", "backlight", "virt-panel", "300")
    assert (
        quick.bluetooth.button.get_active() and quick.bluetooth.state.get_text() == "On"
    )
    quick.bluetooth.button.emit("clicked")
    assert calls[-1] == ("bluetoothctl", "power", "off")
    model.devices = {}
    quick.refresh()
    assert (
        not quick.battery.get_visible()
        and not quick.brightness.box.get_visible()
        and not quick.bluetooth.button.get_sensitive()
    )
print("Laptop battery, brightness units, Bluetooth power and device removal passed.")
