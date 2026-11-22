//! Round-trip editor for `~/.config/mundane/config.toml`.
//!
//! The Compositor and Appearance pages mutate the mundaneDE TOML config in
//! place. `toml_edit::DocumentMut` preserves comments and unrelated keys, so a
//! user's hand-curated config stays intact across panel-driven edits.
//!
//! The mundaneDE compositor itself watches the file via inotify and applies the
//! diff live (see `mundane-de/src/hot_reload.rs`).

use std::path::PathBuf;

use toml_edit::{value, DocumentMut, Item, Table};

use crate::{on_runtime, Error, Result};

/// In-memory representation of `config.toml`. Keep the whole document around so
/// every write is a partial update — comments and unrelated sections survive.
pub struct MundaneConfigFile {
    path: PathBuf,
    doc: DocumentMut,
}

impl MundaneConfigFile {
    /// Path the document was loaded from (or will be written to).
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
        if !doc.contains_key(key) {
            doc.insert(key, Item::Table(Table::new()));
        }
        doc[key].as_table_mut().expect("just-inserted table")
    }

    fn f64(&self, table: &str, key: &str) -> Option<f64> {
        self.doc.get(table)?.get(key)?.as_float().or_else(|| {
            self.doc
                .get(table)?
                .get(key)?
                .as_integer()
                .map(|i| i as f64)
        })
    }

    fn i64(&self, table: &str, key: &str) -> Option<i64> {
        self.doc.get(table)?.get(key)?.as_integer()
    }

    fn str(&self, table: &str, key: &str) -> Option<String> {
        self.doc
            .get(table)?
            .get(key)?
            .as_str()
            .map(|s| s.to_string())
    }

    /// `[spring]` → (stiffness, damping, mass), each defaulting to the compositor defaults
    /// if the key is missing.
    pub fn spring(&self) -> (f64, f64, f64) {
        (
            self.f64("spring", "stiffness").unwrap_or(100.0),
            self.f64("spring", "damping").unwrap_or(20.0),
            self.f64("spring", "mass").unwrap_or(1.0),
        )
    }

    /// `[wm]` → (attraction_radius, outer_gap, inner_gap).
    pub fn wm(&self) -> (i32, i32, i32) {
        let radius = self.i64("wm", "attraction_radius").unwrap_or(24) as i32;
        // outer_gap takes precedence; fall back to the legacy `gap_size` alias.
        let outer = self
            .i64("wm", "outer_gap")
            .or_else(|| self.i64("wm", "gap_size"))
            .unwrap_or(8) as i32;
        let inner = self
            .i64("wm", "inner_gap")
            .map(|v| v as i32)
            .unwrap_or(outer);
        (radius, outer, inner)
    }

    /// `[wallpaper].path`.
    pub fn wallpaper_path(&self) -> Option<String> {
        self.str("wallpaper", "path")
    }

    /// `[wallpaper].mode`, defaulting to "fill".
    pub fn wallpaper_mode(&self) -> String {
        self.str("wallpaper", "mode")
            .unwrap_or_else(|| "fill".into())
    }

    pub fn set_spring(&mut self, stiffness: f64, damping: f64, mass: f64) {
        let t = Self::ensure_table(&mut self.doc, "spring");
        t["stiffness"] = value(stiffness);
        t["damping"] = value(damping);
        t["mass"] = value(mass);
    }

    pub fn set_wm(&mut self, attraction_radius: i32, outer_gap: i32, inner_gap: i32) {
        let t = Self::ensure_table(&mut self.doc, "wm");
        t["attraction_radius"] = value(attraction_radius as i64);
        t["outer_gap"] = value(outer_gap as i64);
        t["inner_gap"] = value(inner_gap as i64);
        // Remove the legacy alias if it's still around so we don't have two
        // keys disagreeing on disk.
        t.remove("gap_size");
    }

    pub fn set_wallpaper(&mut self, path: &str, mode: &str) {
        let t = Self::ensure_table(&mut self.doc, "wallpaper");
        t["path"] = value(path);
        t["mode"] = value(mode);
    }

    pub fn set_wallpaper_mode(&mut self, mode: &str) {
        let t = Self::ensure_table(&mut self.doc, "wallpaper");
        t["mode"] = value(mode);
    }

    /// Render to a string (used by [`save`] and by tests).
    pub fn to_string(&self) -> String {
        self.doc.to_string()
    }
}

/// `$XDG_CONFIG_HOME/mundane/config.toml`, falling back to `~/.config/mundane/config.toml`.
/// Returns `None` only if neither env var is set (e.g. an unusual sandbox).
pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("mundane").join("config.toml"))
}

/// Read the file, parse, return the editable document. A missing file starts an empty
/// document so we can still write to it.
pub async fn load() -> Result<MundaneConfigFile> {
    let path = config_path().ok_or_else(|| Error::Parse("no XDG_CONFIG_HOME or HOME".into()))?;
    on_runtime(async move { read(path) }).await
}

fn read(path: PathBuf) -> Result<MundaneConfigFile> {
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut doc = raw
        .parse::<DocumentMut>()
        .map_err(|e| Error::Parse(e.to_string()))?;
    for section in ["spring", "wm", "wallpaper"] {
        if let Some(item) = doc.get_mut(section) {
            if item.is_inline_table() {
                *item = Item::Table(item.as_inline_table().unwrap().clone().into_table());
            }
            if !item.is_table() {
                return Err(Error::Parse(format!("{section} must be a table")));
            }
        }
    }
    Ok(MundaneConfigFile { path, doc })
}

/// Serialize the entire read/edit/rename operation across Settings processes.
pub async fn update<F>(mutate: F) -> Result<()>
where
    F: FnOnce(&mut MundaneConfigFile) + Send + 'static,
{
    let path = config_path().ok_or_else(|| Error::Parse("no config directory".into()))?;
    on_runtime(async move {
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let dir = path.parent().unwrap();
            std::fs::create_dir_all(dir)?;
            let lock = std::fs::File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path.with_extension("lock"))?;
            lock.lock()?;
            let mut file = read(path.clone())?;
            mutate(&mut file);
            let (k, d, m) = file.spring();
            if !(k.is_finite() && k > 0. && d.is_finite() && d >= 0. && m.is_finite() && m > 0.) {
                return Err(Error::Parse("Invalid spring parameters".into()));
            }
            let mut temp = tempfile::NamedTempFile::new_in(dir)?;
            temp.write_all(file.doc.to_string().as_bytes())?;
            temp.as_file().sync_all()?;
            temp.persist(&path).map_err(|e| Error::Io(e.error))?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Parse(e.to_string()))?
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> MundaneConfigFile {
        MundaneConfigFile {
            path: PathBuf::from("/tmp/test.toml"),
            doc: raw.parse().unwrap(),
        }
    }

    #[test]
    fn reads_spring_with_defaults_for_missing_keys() {
        let f = doc("[spring]\nstiffness = 200.0\n");
        let (s, d, m) = f.spring();
        assert_eq!(s, 200.0);
        assert_eq!(d, 20.0); // default
        assert_eq!(m, 1.0); // default
    }

    #[test]
    fn reads_wm_with_legacy_gap_size_alias() {
        let f = doc("[wm]\nattraction_radius = 32\ngap_size = 12\n");
        let (r, outer, inner) = f.wm();
        assert_eq!(r, 32);
        assert_eq!(outer, 12);
        assert_eq!(inner, 12); // mirrors outer when not specified
    }

    #[test]
    fn reads_wm_with_separate_gaps() {
        let f = doc("[wm]\nattraction_radius = 32\nouter_gap = 12\ninner_gap = 4\n");
        let (r, outer, inner) = f.wm();
        assert_eq!(r, 32);
        assert_eq!(outer, 12);
        assert_eq!(inner, 4);
    }

    #[test]
    fn round_trip_preserves_unrelated_sections_and_comments() {
        let raw = "# heading\n[spring]\nstiffness = 100.0\ndamping = 10.0\nmass = 1.0\n\n[output]\nwidth = 1920\n";
        let mut f = doc(raw);
        f.set_spring(150.0, 12.0, 1.0);
        let out = f.to_string();
        assert!(out.contains("# heading"));
        assert!(out.contains("[output]"));
        assert!(out.contains("stiffness = 150.0"));
        assert!(out.contains("damping = 12.0"));
    }

    #[test]
    fn set_wm_drops_legacy_alias() {
        let raw = "[wm]\nattraction_radius = 24\ngap_size = 8\n";
        let mut f = doc(raw);
        f.set_wm(32, 12, 4);
        let out = f.to_string();
        assert!(!out.contains("gap_size"));
        assert!(out.contains("outer_gap = 12"));
        assert!(out.contains("inner_gap = 4"));
    }

    #[test]
    fn set_wallpaper_writes_path_and_mode() {
        let mut f = doc("");
        f.set_wallpaper("/home/u/wp.png", "fit");
        let out = f.to_string();
        assert!(out.contains("path = \"/home/u/wp.png\""));
        assert!(out.contains("mode = \"fit\""));
    }
}
