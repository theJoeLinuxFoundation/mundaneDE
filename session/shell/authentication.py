"""The shell's polkit agent. The distribution helper owns the PAM conversation."""

import locale
import pwd
import gi

gi.require_version("Polkit", "1.0")
gi.require_version("PolkitAgent", "1.0")
from gi.repository import Gio, GLib, Gtk, Polkit, PolkitAgent

AUTH = "org.freedesktop.PolicyKit1"
AUTH_PATH = "/org/freedesktop/PolicyKit1/Authority"
AGENT_PATH = "/org/joelinux/mundane/AuthenticationAgent"
IFACE = AUTH + ".AuthenticationAgent"
XML = f"""<node><interface name="{IFACE}"><method name="BeginAuthentication">
<arg type="s"/><arg type="s"/><arg type="s"/><arg type="a{{ss}}"/><arg type="s"/><arg type="a(sa{{sv}})"/>
</method><method name="CancelAuthentication"><arg type="s"/></method></interface></node>"""


class Authentication:
    def __init__(self, panel, bus, session_id, conversation=PolkitAgent.Session.new):
        self.panel = panel
        self.bus = bus
        self.subject = ("unix-session", {"session-id": GLib.Variant("s", session_id)})
        self.conversation = conversation
        self.authority = self.reply = self.pam = self.entry = None
        self.cookie = None
        self.locked = False
        self.registration = bus.register_object(
            AGENT_PATH, Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], self.method
        )
        self.watch = Gio.bus_watch_name_on_connection(
            bus, AUTH, 0, self.appeared, self.vanished
        )

    def appeared(self, bus, name, owner):
        bus.call_sync(
            owner,
            AUTH_PATH,
            AUTH + ".Authority",
            "RegisterAuthenticationAgent",
            GLib.Variant(
                "((sa{sv})ss)",
                (self.subject, locale.setlocale(locale.LC_MESSAGES, None), AGENT_PATH),
            ),
            None,
            0,
            5000,
            None,
        )
        self.authority = owner

    def vanished(self, *_):
        self.authority = None
        self.finish(cancel=True)

    def method(self, bus, sender, path, iface, method, args, reply):
        if sender != self.authority:
            reply.return_dbus_error(
                "org.freedesktop.DBus.Error.AccessDenied", "Authority required"
            )
            return
        values = args.unpack()
        if method == "CancelAuthentication":
            if values[0] == self.cookie:
                self.finish(cancel=True)
            reply.return_value(None)
            return
        action, message, _icon, details, cookie, identities = values
        if self.locked or self.reply or not self.panel.outputs:
            reply.return_dbus_error(
                AUTH + ".Error.Cancelled", "Authentication unavailable"
            )
            return
        users = [
            v["uid"] for kind, v in identities if kind == "unix-user" and "uid" in v
        ]
        if not users:
            reply.return_dbus_error(AUTH + ".Error.Failed", "No supported identity")
            return
        box = self.panel.open(next(iter(self.panel.outputs)), "authentication")
        self.panel.reader.sensitive = True
        self.panel.reader.configure()
        self.reply, self.cookie = reply, cookie
        self.panel.menu.set_title("Authentication required")
        box.append(Gtk.Label(label=message, wrap=True))
        info = Gtk.Expander(label="Details")
        info.set_child(
            Gtk.Label(
                label="\n".join(
                    [action, *[f"{key}: {value}" for key, value in details.items()]]
                ),
                wrap=True,
                selectable=True,
            )
        )
        box.append(info)
        names = []
        for uid in users:
            try:
                names.append(pwd.getpwuid(uid).pw_name)
            except KeyError:
                names.append(str(uid))
        choices = Gtk.DropDown.new_from_strings(names)
        choices.update_property([Gtk.AccessibleProperty.LABEL], ["Administrator"])
        box.append(choices)
        self.status = Gtk.Label(wrap=True)
        box.append(self.status)
        self.entry = Gtk.Entry(
            visibility=False, input_purpose=Gtk.InputPurpose.PASSWORD
        )
        self.entry.update_property([Gtk.AccessibleProperty.DESCRIPTION], [message])
        box.append(self.entry)
        self.entry.connect("activate", lambda *_: self.submit())
        for label, callback in [
            ("Authenticate", self.submit),
            ("Cancel", lambda: self.finish(cancel=True)),
        ]:
            control = Gtk.Button(label=label)
            control.connect("clicked", lambda _, cb=callback: cb())
            box.append(control)
        self.panel.menu.present()

        def choose(*_):
            self.reset()
            session = self.conversation(
                Polkit.UnixUser.new(users[choices.get_selected()]), cookie
            )
            self.pam = session
            session.connect("request", self.prompt)
            for signal in ("show-error", "show-info"):
                session.connect(
                    signal,
                    lambda s, text: (
                        self.status.set_text(text) if s is self.pam else None
                    ),
                )
            session.connect("completed", self.completed)
            session.initiate()

        choices.connect("notify::selected", choose)
        choose()

    def prompt(self, session, label, echo):
        if session is self.pam:
            self.entry.set_text("")
            self.entry.set_visibility(echo)
            self.entry.set_placeholder_text(label)
            self.entry.update_property([Gtk.AccessibleProperty.LABEL], [label])
            self.entry.set_sensitive(True)
            self.entry.grab_focus()

    def submit(self):
        if self.pam and self.entry.get_sensitive() and not self.locked:
            answer = self.entry.get_text()
            self.entry.set_text("")
            self.entry.set_sensitive(False)
            self.pam.response(answer)

    def completed(self, session, success):
        if session is self.pam:
            self.pam = None
            if not success:
                self.panel.bus.notify(
                    "Authentication failed", "The action was not authorized."
                )
            self.finish()

    def reset(self):
        if self.entry:
            self.entry.set_text("")
            self.entry.set_sensitive(False)
        session, self.pam = self.pam, None
        if session:
            session.cancel()

    def finish(self, cancel=False):
        if self.reply is None:
            return
        reply, self.reply = self.reply, None
        self.cookie = None
        self.reset()
        self.entry = None
        if self.panel.page == "authentication":
            self.panel.dismiss()
        self.panel.reader.sensitive = False
        self.panel.reader.configure()
        if cancel:
            reply.return_dbus_error(
                AUTH + ".Error.Cancelled", "Authentication cancelled"
            )
        else:
            reply.return_value(None)

    def close(self):
        self.finish(cancel=True)
        if self.authority:
            self.bus.call_sync(
                self.authority,
                AUTH_PATH,
                AUTH + ".Authority",
                "UnregisterAuthenticationAgent",
                GLib.Variant("((sa{sv})s)", (self.subject, AGENT_PATH)),
                None,
                0,
                2000,
                None,
            )
        Gio.bus_unwatch_name(self.watch)
        self.bus.unregister_object(self.registration)
