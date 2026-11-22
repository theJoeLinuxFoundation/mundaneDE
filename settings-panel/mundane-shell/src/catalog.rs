//! Read desktop entries and favourites through the same GIO interfaces as the reference shell.
use gio::{glib, prelude::*};
use gio_unix::DesktopAppInfo;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub class: String,
    pub icon: Option<PathBuf>,
    pub categories: Vec<String>,
    terms: String,
}

impl Application {
    pub fn matches(&self, query: &str) -> bool {
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| self.terms.contains(word))
    }

    pub fn owns(&self, app: &str) -> bool {
        !app.is_empty()
            && (self
                .id
                .trim_end_matches(".desktop")
                .eq_ignore_ascii_case(app.trim_end_matches(".desktop"))
                || self.class.eq_ignore_ascii_case(app))
    }
}

fn category(key: &str) -> Option<&'static str> {
    Some(match key {
        "AudioVideo" | "Video" | "Audio" => "Sound & Video",
        "Development" => "Development",
        "Education" => "Education",
        "Game" => "Games",
        "Graphics" => "Graphics",
        "Network" => "Internet",
        "Office" => "Office",
        "Science" => "Science",
        "Settings" => "Settings",
        "System" => "System",
        "Utility" => "Utilities",
        _ => return None,
    })
}

pub fn icon_path(name: &str, theme: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        return Some(name.into());
    }
    freedesktop_icons::lookup(name)
        .with_size(32)
        .with_theme(theme)
        .with_cache()
        .find()
}

pub fn installed() -> Vec<Application> {
    let theme = gio::Settings::new("org.gnome.desktop.interface").string("icon-theme");
    let mut apps = Vec::new();
    for info in gio::AppInfo::all() {
        if !info.should_show() {
            continue;
        }
        let Ok(app) = info.downcast::<DesktopAppInfo>() else {
            continue;
        };
        let Some(id) = app.id() else { continue };
        let name = app.display_name().to_string();
        let mut categories: Vec<_> = app
            .categories()
            .unwrap_or_default()
            .split(';')
            .filter_map(category)
            .map(String::from)
            .collect();
        categories.sort();
        categories.dedup();
        if categories.is_empty() {
            categories.push("Other".into());
        }
        let terms = format!(
            "{} {} {} {}",
            name,
            id,
            app.generic_name().unwrap_or_default(),
            app.keywords().join(" ")
        )
        .to_lowercase();
        apps.push(Application {
            id: id.to_string(),
            name,
            terms,
            categories,
            class: app.startup_wm_class().unwrap_or_default().to_string(),
            icon: app.string("Icon").and_then(|n| icon_path(&n, &theme)),
        });
    }
    apps.sort_by_cached_key(|app| (app.name.to_lowercase(), app.id.clone()));
    apps
}

pub struct Favourites {
    local: gio::Settings,
    inherited: Option<gio::Settings>,
}
impl Favourites {
    pub fn new() -> Self {
        let inherited = gio::SettingsSchemaSource::default()
            .and_then(|s| s.lookup("org.gnome.shell", true))
            .filter(|s| s.has_key("favorite-apps"))
            .map(|s| gio::Settings::new_full(&s, None::<&gio::SettingsBackend>, None));
        Self {
            local: gio::Settings::new("org.joelinux.mundane.shell"),
            inherited,
        }
    }
    pub fn get(&self) -> Vec<String> {
        let mut source = &self.local;
        if source.user_value("favorite-apps").is_none() {
            if let Some(inherited) = &self.inherited {
                source = inherited;
            }
        }
        let mut seen = std::collections::HashSet::new();
        source
            .strv("favorite-apps")
            .iter()
            .map(ToString::to_string)
            .filter(|s| seen.insert(s.clone()))
            .collect()
    }
    pub fn watch(&self, changed: impl Fn() + Clone + 'static) {
        for prefs in std::iter::once(&self.local).chain(self.inherited.iter()) {
            let changed = changed.clone();
            prefs.connect_changed(Some("favorite-apps"), move |_, _| changed());
        }
    }
    pub fn toggle(&self, id: &str) -> Result<(), glib::BoolError> {
        let mut pins = self.get();
        if pins.iter().any(|p| p == id) {
            pins.retain(|p| p != id);
        } else {
            pins.push(id.into());
        }
        self.local.set_strv("favorite-apps", pins)
    }
}
