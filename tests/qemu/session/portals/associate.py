"""Register the guest test handler when desktop-file-utils is absent on beta1."""

from gi.repository import Gio, GLib
from pathlib import Path

name = "org.joelinux.PortalTest.desktop"
kind = "x-scheme-handler/mundane-portal-test"
path = Path(GLib.get_user_config_dir()) / "mimeapps.list"
config = GLib.KeyFile()
if path.exists():
    config.load_from_file(str(path), GLib.KeyFileFlags.KEEP_COMMENTS)
config.set_string_list("Added Associations", kind, [name])
config.save_to_file(str(path))
assert Gio.AppInfo.get_default_for_type(kind, False).get_id() == name
