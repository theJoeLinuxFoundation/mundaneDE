"""Exercise the shell's polling decisions across observed dave publication states."""

from pathlib import Path
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "session/shell"))
from upgrades import UpgradeNotices


def snapshot(revision, prepared=None, outcome=None):
    last = (
        {
            "event": "deployment-5",
            "deployment": 5,
            "outcome": outcome,
            "message": "Boot check",
        }
        if outcome
        else None
    )
    return dict(
        schema="dave.status/1",
        revision=str(revision),
        managed=True,
        complete=True,
        running=4,
        next_boot=4,
        deployments=[],
        prepared_upgrade=prepared,
        last_upgrade=last,
    )


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "seen"
        shell = UpgradeNotices(path)
        staged = {
            "summary": "2 to install or upgrade, 1 to remove",
            "install": 2,
            "remove": 1,
        }
        assert (
            shell.update({"schema": "dave.plan/1", "upgrade": [{"name": "hello"}]})
            == []
        )
        assert shell.update(snapshot(1)) == []
        assert shell.update(snapshot(2, staged)) == [
            ("Upgrade ready, restart to apply", staged["summary"])
        ]
        shell = UpgradeNotices(path)
        assert (
            shell.update(snapshot(2, staged)) == []
            and staged["summary"] in shell.details
        )
        assert (
            shell.update(dict(snapshot(3), complete=False)) == []
            and shell.seen["prepared"]
        )
        for v in (None, {"schema": "dave.status/9"}, {"schema": "dave.error/1"}):
            assert shell.update(v) == [] and shell.details == [
                "Upgrade status unavailable"
            ]
        for i, outcome in enumerate(
            ("checking", "succeeded", "not_running", "unknown", "not_applied"), 4
        ):
            assert (
                shell.update(snapshot(i, outcome=outcome)) == []
                and not shell.seen["prepared"]
            )
        fallback = snapshot(9, outcome="fallback")
        notices = shell.update(fallback)
        assert len(notices) == 1 and "5" in notices[0][0] and "4" in notices[0][1]
        shell = UpgradeNotices(path)
        assert shell.update(fallback) == [] and "4" in shell.details[1]
        assert shell.update(snapshot(10, staged))
        assert shell.update(dict(snapshot(11), managed=False)) == []
    print(
        "dave readiness, all six boot outcomes, incomplete/unknown data and per-user deduplication passed"
    )
