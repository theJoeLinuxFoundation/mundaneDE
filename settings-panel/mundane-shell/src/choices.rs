//! Translate desktop data into keyed choices. The suite owns filtering, selection and traversal.
use crate::{view::Page, Event, Service, Shell};
use gpui::DisplayId;
use mundane_ui::MenuChoice;
use std::collections::BTreeSet;

impl Page {
    pub(crate) fn choices(&self, shell: &Shell) -> Vec<MenuChoice> {
        match self {
            Page::Start => {
                let groups: BTreeSet<_> = shell
                    .apps
                    .iter()
                    .flat_map(|a| a.categories.iter().cloned())
                    .collect();
                ["Favourites".to_owned(), "All applications".to_owned()]
                    .into_iter()
                    .chain(groups)
                    .map(|s| MenuChoice::new(s.clone(), s))
                    .collect()
            }
            Page::Group(id) => shell
                .tasks
                .iter()
                .filter(|t| shell.apps.iter().any(|a| a.id == *id && a.owns(&t.app)))
                .map(|t| {
                    MenuChoice::new(t.id.clone(), format!("{} · Desk {}", t.title, t.workspace))
                })
                .collect(),
            Page::Tray(_, stack) => {
                let back = (stack.len() > 1).then(|| MenuChoice::new("back", "Back"));
                back.into_iter()
                    .chain(shell.rows.iter().map(|r| {
                        MenuChoice {
                            key: r.id.to_string().into(),
                            label: format!(
                                "{}{}{}",
                                if r.checked { "✓ " } else { "" },
                                r.label,
                                if r.submenu { " ›" } else { "" }
                            )
                            .into(),
                            enabled: r.enabled,
                        }
                    }))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn selection(&self, key: &str, display: DisplayId, shell: &Shell) -> Option<Event> {
        match self {
            Page::Group(_) => Some(Event::Command(format!("activate {key}"))),
            Page::Tray(id, path) if key == "back" && path.len() > 1 => Some(Event::Open(
                display,
                Page::Tray(id.clone(), path[..path.len() - 1].to_vec()),
            )),
            Page::Tray(id, path) => {
                let row = shell
                    .rows
                    .iter()
                    .find(|r| r.enabled && key.parse() == Ok(r.id))?;
                Some(if row.submenu {
                    let mut path = path.clone();
                    path.push(row.id);
                    Event::Open(display, Page::Tray(id.clone(), path))
                } else {
                    Event::Service(Service::Select(id.clone(), row.id))
                })
            }
            _ => None,
        }
    }
}
