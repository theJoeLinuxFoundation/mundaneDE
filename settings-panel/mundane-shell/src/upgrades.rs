//! Consume dave's published status and share deduplication state with the reference session.
use crate::{Event, Service};
use async_channel::Sender;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command, time::Duration};

#[derive(Deserialize)]
struct Status {
    schema: String,
    complete: bool,
    revision: String,
    managed: bool,
    running: Option<u64>,
    next_boot: Option<u64>,
    deployments: Vec<Deployment>,
    prepared_upgrade: Option<Prepared>,
    last_upgrade: Option<Last>,
}
#[derive(Deserialize)]
struct Deployment {
    id: u64,
    health: Option<String>,
}
#[derive(Deserialize)]
struct Prepared {
    id: Option<String>,
    summary: String,
    install: u64,
    remove: u64,
}
#[derive(Deserialize)]
struct Last {
    event: String,
    deployment: Option<u64>,
    outcome: String,
    #[serde(default)]
    message: String,
}
#[derive(Default)]
struct Poll {
    revision: Option<String>,
    details: Vec<String>,
    seen: Value,
}

fn remember(seen: &mut Value, key: &str, id: &str) -> bool {
    if !seen[key].is_array() {
        seen[key] = json!([]);
    }
    let ids = seen[key].as_array_mut().unwrap();
    if ids.iter().any(|x| x.as_str() == Some(id)) {
        return false;
    }
    ids.push(json!(id));
    true
}
impl Poll {
    fn update(&mut self, s: Option<Status>) -> Vec<(String, String)> {
        let mut alerts = Vec::new();
        let Some(s) =
            s.filter(|s| s.schema == "dave.status/1" && s.complete && !s.revision.is_empty())
        else {
            self.revision = None;
            self.details = vec!["Upgrade status unavailable".into()];
            return alerts;
        };
        if self.revision.as_ref() == Some(&s.revision) {
            return alerts;
        }
        self.revision = Some(s.revision);
        self.details.clear();
        if !s.managed {
            if !self.seen["prepared"].is_null() {
                self.seen["prepared"] = Value::Null;
            }
            return alerts;
        }
        if let Some(p) = s.prepared_upgrade {
            let title = "Upgrade ready, restart to apply";
            self.details.extend([title.into(), p.summary.clone()]);
            let legacy = json!([p.summary, p.install, p.remove]);
            let was_legacy = self.seen["prepared"] == legacy;
            let fresh = match p.id.as_deref() {
                Some(id) => remember(&mut self.seen, "prepared_ids", id) && !was_legacy,
                None => !was_legacy,
            };
            self.seen["prepared"] = p.id.map(Value::String).unwrap_or(legacy);
            if fresh {
                alerts.push((title.into(), p.summary));
            }
        } else {
            self.seen["prepared"] = Value::Null;
        }
        if let Some(l) = s.last_upgrade {
            if l.outcome == "fallback" {
                if let (Some(failed), Some(restored)) = (l.deployment, s.running) {
                    let title = format!("Upgrade to deployment {failed} failed");
                    let body = format!("Returned to deployment {restored}. {}", l.message)
                        .trim_end()
                        .to_string();
                    self.details.extend([title.clone(), body.clone()]);
                    if remember(&mut self.seen, "fallback_events", &l.event) {
                        alerts.push((title, body));
                    }
                }
            } else if l.outcome == "not_applied" {
                self.details
                    .push(format!("Upgrade could not be applied: {}", l.message));
            }
        }
        for (label, id) in [("Running", s.running), ("Next boot", s.next_boot)] {
            if let Some(id) = id {
                let health = s
                    .deployments
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.health.as_ref())
                    .map(|h| format!(" ({h})"))
                    .unwrap_or_default();
                self.details
                    .push(format!("{label}: deployment {id}{health}"));
            }
        }
        alerts
    }
}
pub fn start(tx: Sender<Event>, service: Sender<Service>) {
    std::thread::spawn(move || {
        let dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| gio::glib::home_dir().join(".local/state"))
            .join("mundane");
        let path = dir.join("upgrade-notices.json");
        let mut poll = Poll {
            seen: fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .filter(|v| v.is_object())
                .unwrap_or(json!({})),
            ..Default::default()
        };
        loop {
            let status = Command::new("dave")
                .args(["status", "--json"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| serde_json::from_slice(&o.stdout).ok());
            let old = poll.details.clone();
            let seen = poll.seen.clone();
            let alerts = poll.update(status);
            if seen != poll.seen {
                if let Err(e) = save(&path, &poll.seen) {
                    eprintln!("Save upgrade notice state: {e}");
                }
            }
            if old != poll.details
                && tx
                    .send_blocking(Event::UpgradeDetails(poll.details.clone()))
                    .is_err()
            {
                return;
            }
            for (t, b) in alerts {
                if service.send_blocking(Service::LocalNotice(t, b)).is_err() {
                    return;
                }
            }
            std::thread::sleep(Duration::from_secs(15));
        }
    });
}
fn save(path: &std::path::Path, value: &Value) -> anyhow::Result<()> {
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new_in(dir)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    f.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(rev: u32, prepared: Value, outcome: &str) -> Option<Status> {
        serde_json::from_value(json!({"schema":"dave.status/1","revision":rev.to_string(),
            "managed":true,"complete":true,"running":4,"next_boot":4,
            "deployments":[{"id":4,"health":"good"}],"prepared_upgrade":prepared,
            "last_upgrade":{"deployment":5,"event":"upgrade-five","outcome":outcome,"message":"Boot check"}})).ok()
    }

    #[test]
    fn prepared_ids_and_fallback_events_survive_restart() {
        let mut p = Poll {
            seen: json!({}),
            ..Default::default()
        };
        let staged = json!({"id":"sha256:a","summary":"Two packages","install":2,"remove":0});
        assert_eq!(p.update(status(1, staged.clone(), "succeeded")).len(), 1);
        let mut p = Poll {
            seen: p.seen,
            ..Default::default()
        };
        assert!(p.update(status(1, staged.clone(), "succeeded")).is_empty());
        let mut other = staged.clone();
        other["id"] = json!("sha256:b");
        assert_eq!(p.update(status(2, other, "checking")).len(), 1);
        assert!(p.update(None).is_empty());
        assert!(p.update(status(3, staged, "not_running")).is_empty());
        let alerts = p.update(status(4, Value::Null, "fallback"));
        assert_eq!(
            alerts,
            vec![(
                "Upgrade to deployment 5 failed".into(),
                "Returned to deployment 4. Boot check".into()
            )]
        );
        let mut p = Poll {
            seen: p.seen,
            ..Default::default()
        };
        assert!(p.update(status(4, Value::Null, "fallback")).is_empty());
        assert!(p
            .details
            .iter()
            .any(|s| s == "Running: deployment 4 (good)"));
        for (i, outcome) in [
            "not_applied",
            "unknown",
            "succeeded",
            "checking",
            "not_running",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(p
                .update(status(i as u32 + 5, Value::Null, outcome))
                .is_empty());
        }
    }
    #[test]
    fn preserve_pre_id_delivery_and_ignore_incomplete_snapshots() {
        let mut p = Poll {
            seen: json!({"prepared":["Two packages",2,0]}),
            ..Default::default()
        };
        let mut s = status(
            1,
            json!({"id":"sha256:a","summary":"Two packages","install":2,"remove":0}),
            "succeeded",
        )
        .unwrap();
        s.complete = false;
        assert!(p.update(Some(s)).is_empty());
        assert_eq!(p.details, ["Upgrade status unavailable"]);
        assert!(p
            .update(status(
                2,
                json!({"id":"sha256:a","summary":"Two packages","install":2,"remove":0}),
                "succeeded"
            ))
            .is_empty());
        assert_eq!(p.seen["prepared_ids"], json!(["sha256:a"]));
    }
}
