//! User selections returned to xdg-desktop-portal. The frontend exports document grants.
use crate::{
    cast::Props,
    view::{shell_button, text_button},
    Event, Service, Shell,
};
use gio::{glib, prelude::*};
use gpui::{prelude::*, *};
use mundane_ui::{tokens, TextField, Theme};
use std::{
    collections::{BTreeSet, HashMap},
    ffi::{CString, OsStr},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Request {
    pub id: u64,
    pub method: String,
    pub title: String,
    pub apps: Vec<String>,
    pub options: Props,
}
type Filter = (String, Vec<(u32, String)>);
type Choice = (String, String, Vec<(String, String)>, String);
fn bytes_path(bytes: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(bytes.strip_suffix(&[0]).unwrap_or(bytes)))
}
fn option_path(request: &Request, key: &str) -> Option<PathBuf> {
    request
        .options
        .get(key)?
        .get::<Vec<u8>>()
        .map(|b| bytes_path(&b).to_owned())
        .filter(|p| p.is_absolute())
}
fn matches_filter(path: &Path, filter: &Filter) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    filter.1.iter().any(|(kind, pattern)| match kind {
        0 => CString::new(pattern.as_bytes())
            .ok()
            .zip(CString::new(name.as_bytes()).ok())
            .is_some_and(|(pat, name)| unsafe {
                libc::fnmatch(pat.as_ptr(), name.as_ptr(), 0) == 0
            }),
        1 => {
            let (mime, _) = gio::content_type_guess(Some(path), None);
            pattern.strip_suffix("/*").map_or_else(
                || gio::content_type_is_a(&mime, pattern),
                |prefix| mime.split('/').next() == Some(prefix),
            )
        }
        _ => false,
    })
}

pub struct Dialog {
    req: Request,
    dir: PathBuf,
    entries: Vec<PathBuf>,
    field: Entity<TextField>,
    selected: BTreeSet<PathBuf>,
    filters: Vec<Filter>,
    filter: usize,
    options: Vec<Choice>,
    error: String,
    overwrite: Option<PathBuf>,
    focus: HashMap<String, FocusHandle>,
    cancel: FocusHandle,
    submit: FocusHandle,
}
impl Dialog {
    pub fn new(req: Request, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = option_path(&req, "current_file");
        let dir = current
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_owned)
            .or_else(|| option_path(&req, "current_folder"))
            .filter(|p| p.is_dir())
            .unwrap_or_else(glib::home_dir);
        let initial = current
            .and_then(|p| p.file_name().map(|v| v.to_string_lossy().into_owned()))
            .or_else(|| req.options.get("current_name")?.get::<String>())
            .unwrap_or_default();
        let theme = Theme::current(cx);
        let field =
            cx.new(|cx| TextField::new("Location or filename", theme, cx).select_value(initial));
        let mut filters = req
            .options
            .get("filters")
            .and_then(|v| v.get::<Vec<Filter>>())
            .unwrap_or_default();
        let mut filter = 0;
        if let Some(f) = req
            .options
            .get("current_filter")
            .and_then(|v| v.get::<Filter>())
        {
            filter = filters.iter().position(|x| x == &f).unwrap_or_else(|| {
                filters.push(f);
                filters.len() - 1
            });
        }
        let mut options = req
            .options
            .get("choices")
            .and_then(|v| v.get::<Vec<Choice>>())
            .unwrap_or_default();
        for (_, _, values, current) in &mut options {
            if current.is_empty() {
                *current = values
                    .first()
                    .map_or_else(|| "false".into(), |v| v.0.clone());
            }
        }
        let cancel = cx.focus_handle().tab_stop(true);
        cancel.focus(w, cx);
        let mut this = Self {
            req,
            dir,
            entries: vec![],
            field,
            selected: Default::default(),
            filters,
            filter,
            options,
            error: String::new(),
            overwrite: None,
            focus: HashMap::new(),
            cancel,
            submit: cx.focus_handle().tab_stop(true),
        };
        this.read_dir();
        this
    }
    fn directory(&self) -> bool {
        self.req.method == "SaveFiles"
            || self
                .req
                .options
                .get("directory")
                .and_then(|v| v.get::<bool>())
                == Some(true)
    }
    fn read_dir(&mut self) {
        if self.req.method == "ChooseApplication" {
            return;
        }
        self.focus.clear();
        self.entries.clear();
        self.selected.clear();
        self.error.clear();
        self.overwrite = None;
        match self.dir.read_dir() {
            Ok(items) => {
                self.entries = items
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|f| !f.as_bytes().starts_with(b"."))
                            && (p.is_dir()
                                || (!self.directory()
                                    && self
                                        .filters
                                        .get(self.filter)
                                        .is_none_or(|f| matches_filter(p, f))))
                    })
                    .collect();
                self.entries.sort_by_cached_key(|p| {
                    (
                        !p.is_dir(),
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase(),
                    )
                });
            }
            Err(e) => self.error = e.to_string(),
        }
    }
    fn reset_field(&mut self, text: String, cx: &mut Context<Self>) {
        let t = Theme::current(cx);
        self.field = cx.new(|cx| TextField::new("Location or filename", t, cx).select_value(text));
        self.overwrite = None;
    }
    fn accept(&mut self, cx: &mut Context<Self>) {
        self.error.clear();
        let typed = self.field.read(cx).text();
        let path = self.dir.join(typed);
        let mut paths = Vec::new();
        if self.req.method == "SaveFiles" {
            if !path.is_dir() {
                self.error = "Choose a folder.".into();
                return;
            }
            let names = self
                .req
                .options
                .get("files")
                .and_then(|v| v.get::<Vec<Vec<u8>>>())
                .unwrap_or_default();
            for name in names {
                let Some(name) = bytes_path(&name).file_name() else {
                    self.error = "Invalid filename.".into();
                    return;
                };
                let mut candidate = path.join(name);
                let mut n = 1;
                while candidate.exists() || paths.contains(&candidate) {
                    let file = Path::new(name);
                    let mut base = file.file_stem().unwrap_or(name).to_owned();
                    base.push(format!(" ({n})"));
                    if let Some(ext) = file.extension() {
                        base.push(".");
                        base.push(ext);
                    }
                    candidate = path.join(base);
                    n += 1;
                }
                paths.push(candidate);
            }
        } else if self.req.method == "SaveFile" {
            if typed.is_empty() || path.is_dir() || !path.parent().is_some_and(Path::is_dir) {
                self.error = "Enter a filename in an existing folder.".into();
                return;
            }
            if path.exists() && self.overwrite.as_ref() != Some(&path) {
                self.error = format!("Replace {}?", path.display());
                self.overwrite = Some(path);
                return;
            }
            paths.push(path);
        } else {
            paths = if typed.is_empty() && !self.selected.is_empty() {
                self.selected.iter().cloned().collect()
            } else {
                vec![path]
            };
            for p in &paths {
                if !p.exists()
                    || p.is_dir() != self.directory()
                    || (!p.is_dir()
                        && self
                            .filters
                            .get(self.filter)
                            .is_some_and(|f| !matches_filter(p, f)))
                {
                    self.error = "Select an item matching the file type.".into();
                    return;
                }
            }
        }
        if paths.is_empty() {
            self.error = "Select a file.".into();
            return;
        }
        let mut result = Props::new();
        result.insert(
            "uris".into(),
            paths
                .iter()
                .map(|p| gio::File::for_path(p).uri().to_string())
                .collect::<Vec<_>>()
                .to_variant(),
        );
        if let Some(f) = self.filters.get(self.filter) {
            result.insert("current_filter".into(), f.to_variant());
        }
        result.insert(
            "choices".into(),
            self.options
                .iter()
                .map(|(k, _, _, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>()
                .to_variant(),
        );
        self.finish(result, cx);
    }
    fn finish(&self, result: Props, cx: &App) {
        let _ = cx
            .global::<Shell>()
            .events
            .try_send(Event::Service(Service::Chosen(self.req.id, result)));
    }
}
impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::current(cx);
        let mut root = div()
            .flex()
            .flex_col()
            .gap(tokens::space::SM)
            .min_h_0()
            .h_full();
        let mut list = div()
            .id("items")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let view = cx.weak_entity();
        if self.req.method == "ChooseApplication" {
            if let Some(r) = &cx.global::<Shell>().choice {
                self.req.apps = r.apps.clone();
            }
            for app in &self.req.apps {
                let label = cx
                    .global::<Shell>()
                    .apps
                    .iter()
                    .find(|a| a.id.trim_end_matches(".desktop") == app.trim_end_matches(".desktop"))
                    .map_or_else(|| app.clone(), |a| a.name.clone());
                let key = app.clone();
                let ent = view.clone();
                let f = self
                    .focus
                    .entry(key.clone())
                    .or_insert_with(|| cx.focus_handle().tab_stop(true));
                list = list.child(shell_button(label, f, t, move |_, cx| {
                    let _ = ent.update(cx, |v, cx| {
                        v.finish(Props::from([("choice".into(), key.to_variant())]), cx)
                    });
                }));
            }
        } else {
            let parent = view.clone();
            let up = self
                .focus
                .entry("parent".into())
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
            root = root
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(tokens::space::SM)
                        .child(text_button("Up", up, t, move |_, cx| {
                            let _ = parent.update(cx, |v, cx| {
                                v.dir.pop();
                                v.read_dir();
                                v.reset_field(String::new(), cx);
                                cx.notify();
                            });
                        }))
                        .child(self.dir.display().to_string()),
                )
                .child(self.field.clone());
            for path in &self.entries {
                let uri = path.to_string_lossy().into_owned();
                let label = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let directory = path.is_dir();
                let p = path.clone();
                let ent = view.clone();
                let handle = self
                    .focus
                    .entry(uri)
                    .or_insert_with(|| cx.focus_handle().tab_stop(true));
                let selected = self.selected.contains(path);
                let row = mundane_ui::launcher_row(
                    label.clone(),
                    handle,
                    selected,
                    true,
                    t,
                    move |_, cx| {
                        let _ = ent.update(cx, |v, cx| {
                            if directory {
                                v.dir = p.clone();
                                v.read_dir();
                                v.reset_field(String::new(), cx);
                            } else if v.req.method == "SaveFile" {
                                v.reset_field(
                                    p.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned(),
                                    cx,
                                );
                            } else {
                                if v.req.options.get("multiple").and_then(|v| v.get::<bool>())
                                    != Some(true)
                                {
                                    v.selected.clear();
                                }
                                if !v.selected.remove(&p) {
                                    v.selected.insert(p.clone());
                                }
                                v.reset_field(String::new(), cx);
                            }
                            cx.notify();
                        });
                    },
                );
                list = list.child(row.child(if directory {
                    format!("{label}/")
                } else {
                    label
                }));
            }
        }
        root = root.child(list);
        if let Some(f) = self.filters.get(self.filter) {
            let handle = self
                .focus
                .entry("filter".into())
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
            let ent = view.clone();
            root = root.child(shell_button(
                format!("File type: {}", f.0),
                handle,
                t,
                move |_, cx| {
                    let _ = ent.update(cx, |v, cx| {
                        v.filter = (v.filter + 1) % v.filters.len();
                        v.read_dir();
                        cx.notify();
                    });
                },
            ));
        }
        for (index, (key, label, values, selected)) in self.options.iter().enumerate() {
            let handle = self
                .focus
                .entry(format!("option:{key}"))
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
            let value = values
                .iter()
                .find(|v| v.0 == *selected)
                .map_or(selected.as_str(), |x| x.1.as_str());
            let ent = view.clone();
            root = root.child(shell_button(
                format!("{label}: {value}"),
                handle,
                t,
                move |_, cx| {
                    let _ = ent.update(cx, |v, cx| {
                        let (_, _, options, current) = &mut v.options[index];
                        *current = if options.is_empty() {
                            (current != "true").to_string()
                        } else {
                            options[(options
                                .iter()
                                .position(|o| o.0 == *current)
                                .unwrap_or_default()
                                + 1)
                                % options.len()]
                            .0
                            .clone()
                        };
                        cx.notify();
                    });
                },
            ));
        }
        if !self.error.is_empty() {
            root = root.child(div().text_color(rgb(t.error())).child(self.error.clone()));
        }
        let events = cx.global::<Shell>().events.clone();
        let mut buttons = div()
            .flex()
            .justify_end()
            .gap(tokens::space::SM)
            .child(text_button("Cancel", &self.cancel, t, move |_, _| {
                let _ = events.try_send(Event::Dismiss);
            }));
        if self.req.method != "ChooseApplication" {
            let label = self
                .req
                .options
                .get("accept_label")
                .and_then(|v| v.get::<String>())
                .unwrap_or_else(|| {
                    if self.req.method.starts_with("Save") {
                        "Save".into()
                    } else {
                        "Open".into()
                    }
                })
                .replace('_', "");
            buttons = buttons.child(shell_button(
                if self.overwrite.is_some() {
                    "Replace".into()
                } else {
                    label
                },
                &self.submit,
                t,
                move |_, cx| {
                    let _ = view.update(cx, |v, cx| {
                        v.accept(cx);
                        cx.notify();
                    });
                },
            ));
        }
        root.child(buttons)
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, w, cx| {
                if e.keystroke.modifiers.control && e.keystroke.key == "l" {
                    v.field.read(cx).focus_handle(cx).focus(w, cx);
                    cx.stop_propagation();
                }
                if e.keystroke.key == "enter" && v.field.read(cx).focus_handle(cx).is_focused(w) {
                    v.accept(cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::{bytes_path, matches_filter};
    use std::{os::unix::ffi::OsStrExt, path::Path};

    #[test]
    fn portal_filters_accept_any_matching_rule() {
        let filter = (
            "Documents".into(),
            vec![(0, "*.md".into()), (1, "image/*".into())],
        );
        assert!(matches_filter(Path::new("résumé.md"), &filter));
        assert!(matches_filter(Path::new("landscape.png"), &filter));
        assert!(!matches_filter(Path::new("system.log"), &filter));
    }

    #[test]
    fn portal_paths_preserve_non_utf8_names_and_drop_the_terminator() {
        let raw = b"/tmp/example-\xff\0";
        assert_eq!(
            bytes_path(raw).as_os_str().as_bytes(),
            &raw[..raw.len() - 1]
        );
    }
}
