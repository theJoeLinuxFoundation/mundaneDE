"""A private IBus engine. J opens Japanese candidates, K commits the first."""

import sys
import gi

gi.require_version("IBus", "1.0")
from gi.repository import GLib, IBus


class TestEngine(IBus.Engine):
    composing = False

    def do_process_key_event(self, key, code, state):
        if state & IBus.ModifierType.RELEASE_MASK:
            return False
        if key == ord("j"):
            self.composing = True
            self.update_preedit_text(IBus.Text.new_from_string("に"), 1, True)
            table = IBus.LookupTable.new(5, 0, True, False)
            for candidate in ["日本", "二"]:
                table.append_candidate(IBus.Text.new_from_string(candidate))
            self.update_lookup_table(table, True)
            return True
        if key == ord("k") and self.composing:
            self.commit_text(IBus.Text.new_from_string("日本"))
            self.do_reset()
            return True
        return False

    def do_reset(self):
        self.composing = False
        self.hide_preedit_text()
        self.hide_lookup_table()

    def do_focus_out(self):
        self.do_reset()


IBus.init()
if "--select" in sys.argv:
    assert IBus.Bus().set_global_engine("mundane-test")
    raise SystemExit
bus = IBus.Bus()
factory = IBus.Factory.new(bus.get_connection())
factory.add_engine("mundane-test", TestEngine.__gtype__)
component = IBus.Component.new("org.freedesktop.IBus.MundaneTest", "Test", "1", "BSD", "", "", "", "")
component.add_engine(
    IBus.EngineDesc.new("mundane-test", "Mundane Test", "Fixture", "ja", "BSD", "", "", "us")
)
assert bus.register_component(component)
print("engine registered", flush=True)
GLib.MainLoop().run()
