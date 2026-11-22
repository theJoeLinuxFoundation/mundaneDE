"""Installed desktop entries, launcher search and the user's ordered favourites."""

from gi.repository import Gio

CATEGORIES = {
    "AudioVideo": "Sound & Video",
    "Audio": "Sound & Video",
    "Video": "Sound & Video",
    "Development": "Development",
    "Education": "Education",
    "Game": "Games",
    "Graphics": "Graphics",
    "Network": "Internet",
    "Office": "Office",
    "Science": "Science",
    "Settings": "Settings",
    "System": "System",
    "Utility": "Utilities",
}


class Applications:
    def __init__(self, changed):
        source = Gio.SettingsSchemaSource.get_default()
        self.prefs = Gio.Settings.new("org.joelinux.mundane.shell")
        # Inherit joe's defaults and the user's existing order until the first local edit.
        gnome = source.lookup("org.gnome.shell", True)
        self.gnome = (
            Gio.Settings.new_full(gnome, None, None)
            if gnome and gnome.has_key("favorite-apps")
            else None
        )
        self.changed = changed
        self.load()
        self.monitor = Gio.AppInfoMonitor.get()
        self.monitor.connect("changed", self.reload)
        self.prefs.connect("changed::favorite-apps", lambda *_: changed())
        if self.gnome:
            self.gnome.connect("changed::favorite-apps", lambda *_: changed())

    def reload(self, *_):
        self.load()
        self.changed()

    def load(self):
        self.apps = sorted(
            (
                a
                for a in Gio.AppInfo.get_all()
                if isinstance(a, Gio.DesktopAppInfo) and a.should_show()
            ),
            key=lambda a: (a.get_display_name().casefold(), a.get_id()),
        )
        self.by_id = {a.get_id(): a for a in self.apps}
        self.lookup = {}
        self.groups = {}
        self.terms = {}
        for app in self.apps:
            ident = app.get_id()
            self.lookup[ident.removesuffix(".desktop").casefold()] = ident
            wm_class = app.get_startup_wm_class()
            if wm_class:
                self.lookup.setdefault(wm_class.casefold(), ident)
            categories = {
                CATEGORIES[k]
                for k in (app.get_categories() or "").split(";")
                if k in CATEGORIES
            }
            for name in categories or {"Other"}:
                self.groups.setdefault(name, []).append(app)
            self.terms[ident] = " ".join(
                [
                    app.get_display_name(),
                    ident,
                    app.get_generic_name() or "",
                    *(app.get_keywords() or []),
                ]
            ).casefold()

    def pinned(self):
        prefs = self.prefs
        if self.gnome and prefs.get_user_value("favorite-apps") is None:
            prefs = self.gnome
        return list(dict.fromkeys(prefs.get_strv("favorite-apps")))

    def toggle(self, ident):
        pins = self.pinned()
        if ident in pins:
            pins.remove(ident)
        else:
            pins.append(ident)
        self.prefs.set_strv("favorite-apps", pins)

    def matches(self, category, query):
        words = query.casefold().split()
        if words:
            matches = (
                a for a in self.apps if all(w in self.terms[a.get_id()] for w in words)
            )
            return sorted(
                matches,
                key=lambda a: not a.get_display_name().casefold().startswith(words[0]),
            )
        if category == "Favourites":
            return [self.by_id[i] for i in self.pinned() if i in self.by_id]
        return self.groups.get(category, self.apps)
