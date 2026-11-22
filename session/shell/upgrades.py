"""User-visible deployment state from the published dave.status/1 snapshot."""

import json
import os
from pathlib import Path
import tempfile


class UpgradeNotices:
    def __init__(self, path):
        self.path = Path(path)
        self.revision = None
        self.details = []
        try:
            self.seen = json.loads(self.path.read_text())
            if not isinstance(self.seen, dict):
                self.seen = {}
        except (OSError, ValueError):
            self.seen = {}

    def save(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(
            mode="w", dir=self.path.parent, delete=False
        ) as f:
            json.dump(self.seen, f)
        os.replace(f.name, self.path)

    def update(self, status):
        if (
            not isinstance(status, dict)
            or status.get("schema") != "dave.status/1"
            or not status.get("complete")
            or not status.get("revision")
        ):
            self.revision = None
            self.details = ["Upgrade status unavailable"]
            return []
        if status["revision"] == self.revision:
            return []
        self.revision = status["revision"]
        self.details = []
        alerts = []
        if not status["managed"]:
            if self.seen.get("prepared"):
                self.seen["prepared"] = False
                self.save()
            return []
        prepared = status["prepared_upgrade"]
        stamp = None
        updated = dict(self.seen)
        if prepared is not None:
            title = "Upgrade ready, restart to apply"
            self.details.extend([title, prepared["summary"]])
            legacy = [prepared["summary"], prepared["install"], prepared["remove"]]
            stamp = prepared.get("id") or legacy
            if isinstance(stamp, str):
                delivered = self.seen.get("prepared_ids", [])
                fresh = stamp not in delivered and self.seen.get("prepared") != legacy
                if stamp not in delivered:
                    updated["prepared_ids"] = [*delivered, stamp]
            else:
                fresh = self.seen.get("prepared") != legacy
            if fresh:
                alerts.append((title, prepared["summary"]))
        updated["prepared"] = stamp
        last = status["last_upgrade"]
        if last and last["outcome"] == "fallback":
            title = f"Upgrade to deployment {last['deployment']} failed"
            body = f"Returned to deployment {status['running']}."
            if last.get("message"):
                body += " " + last["message"]
            self.details.extend([title, body])
            delivered = self.seen.get("fallback_events", [])
            if last["event"] not in delivered:
                alerts.append((title, body))
                updated["fallback_events"] = [*delivered, last["event"]]
        elif last and last["outcome"] == "not_applied":
            self.details.append("Upgrade could not be applied: " + last["message"])
        deployments = {d["id"]: d.get("health") for d in status["deployments"]}
        for label, key in [("Running", "running"), ("Next boot", "next_boot")]:
            ident = status[key]
            if ident is not None:
                health = deployments.get(ident)
                self.details.append(
                    f"{label}: deployment {ident}" + (f" ({health})" if health else "")
                )
        if updated != self.seen:
            self.seen = updated
            self.save()
        return alerts
