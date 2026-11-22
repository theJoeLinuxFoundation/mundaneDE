"""Run with dbus-run-session and a private HOME. No display is required."""

import os
from pathlib import Path
import subprocess
import sys
import time
from gi.repository import GLib

here = Path(__file__).resolve().parent
sys.path.insert(0, str(here.parents[2] / "session/shell"))
from bluetooth import Bluetooth
from devices import battery_status

assert "DISPLAY" not in os.environ and "WAYLAND_DISPLAY" not in os.environ
os.environ["DBUS_SYSTEM_BUS_ADDRESS"] = os.environ["DBUS_SESSION_BUS_ADDRESS"]
record = Path(os.environ["HOME"]) / "events"


class Panel:
    page = None
    accept = True
    alerts = []

    def notify(self, title, body):
        self.alerts.append(title)

    def pairing(self, method, values, answer):
        assert method == "RequestConfirmation" and values[1] == 123456
        answer(None if self.accept else False)


def wait(check):
    end = time.monotonic() + 6
    while time.monotonic() < end:
        while GLib.MainContext.default().iteration(False):
            pass
        if check():
            return
        time.sleep(0.02)
    raise AssertionError("Timed out waiting for fixture")


proc = subprocess.Popen([sys.executable, here / "device_fixture.py", record])
try:
    panel = Panel()
    panel.bus = panel
    bt = Bluetooth(panel)
    wait(lambda: bt.owner and battery_status() == "Battery 72%")
    for answer, label in [(False, "cancelled"), (True, "paired")]:
        panel.accept = answer
        bt.pair("11:22:33:44:55:66")
        wait(
            lambda: (
                record.exists()
                and label in record.read_text().splitlines()
                and bt.device is None
            )
        )
    bt.pair("00:00:00:00:00:00")
    assert "Bluetooth pairing unavailable" in panel.alerts
finally:
    proc.terminate()
    proc.wait()
wait(lambda: bt.owner is None)
assert battery_status() == "Battery status unavailable"
print(
    "Private BlueZ confirmation, cancellation, disappearing device and UPower percentage passed"
)
