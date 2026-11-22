"""Exercise source selection without compiling or opening a QEMU window."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


class CommittedLauncher(unittest.TestCase):
    def test_revision_and_overlay_selection(self):
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            origin = work / "upstream"
            local = work / "developer"
            origin.mkdir()
            env = {
                **os.environ,
                "XDG_CACHE_HOME": str(work / "cache"),
                "MUNDANE_TRY_DIR": str(work / "qualification"),
                "GIT_AUTHOR_NAME": "Fixture",
                "GIT_COMMITTER_NAME": "Fixture",
                "GIT_AUTHOR_EMAIL": "test@example.invalid",
                "GIT_COMMITTER_EMAIL": "test@example.invalid",
            }

            def run(cwd, *args, check=True):
                return subprocess.run(
                    args, cwd=cwd, env=env, text=True, capture_output=True, check=check
                )

            run(origin, "git", "init", "-b", "main")
            driver = origin / "tests/qemu/session"
            driver.mkdir(parents=True)
            shutil.copy(HERE / "try_revision.py", driver)
            (origin / "settings-panel").mkdir()
            (origin / "settings-panel/Cargo.toml").touch()
            (driver / "try_session.py").write_text(
                """from pathlib import Path
import argparse, os
p = argparse.ArgumentParser()
p.add_argument('--source', type=Path)
p.add_argument('--build-label')
args, _ = p.parse_known_args()
print('title', args.build_label)
print('guest', os.environ['MUNDANE_TRY_DIR'], (args.source/'message').read_text())
"""
            )
            (origin / "message").write_text("one")
            run(origin, "git", "add", ".")
            run(origin, "git", "commit", "-m", "first")
            first = run(origin, "git", "rev-parse", "HEAD").stdout.strip()
            run(work, "git", "clone", str(origin), str(local))
            launcher = [sys.executable, "tests/qemu/session/try_revision.py"]
            (origin / "message").write_text("two")
            run(origin, "git", "commit", "-am", "second")
            latest = run(origin, "git", "rev-parse", "HEAD").stdout.strip()
            output = run(local, *launcher, "--headless").stdout
            self.assertIn(latest, output)
            self.assertTrue(output.strip().endswith(" two"), output)
            self.assertTrue(
                run(local, *launcher, "--revision", first)
                .stdout.strip()
                .endswith(" one")
            )
            (local / "message").write_text("unfinished")
            self.assertEqual(run(local, *launcher, check=False).returncode, 2)
            self.assertTrue(
                run(local, *launcher, "--from-working-tree")
                .stdout.strip()
                .endswith(" unfinished")
            )
            run(local, "git", "commit", "-am", "local only")
            unpublished = run(local, "git", "rev-parse", "HEAD").stdout.strip()
            for flags in ([], ["--revision", first]):
                self.assertEqual(
                    run(local, *launcher, *flags, check=False).returncode, 2
                )
            opted_in = run(local, *launcher, "--from-working-tree").stdout
            self.assertIn("title " + unpublished + " (working tree, clean)", opted_in)
            run(local, "git", "switch", "--detach", first)
            self.assertEqual(
                run(
                    local, *launcher, "--revision", unpublished, check=False
                ).returncode,
                2,
            )
            self.assertIn("title " + latest, run(local, *launcher).stdout)
            run(local, "git", "push", "origin", unpublished + ":refs/heads/candidate")
            self.assertIn(
                "title " + unpublished,
                run(local, *launcher, "--revision", unpublished).stdout,
            )
            run(local, "git", "push", "origin", "--delete", "candidate")
            self.assertEqual(
                run(
                    local, *launcher, "--revision", unpublished, check=False
                ).returncode,
                2,
            )
            run(local, "git", "tag", "-a", "review", unpublished, "-m", "review")
            run(local, "git", "push", "origin", "refs/tags/review")
            self.assertIn(
                "title " + unpublished,
                run(local, *launcher, "--revision", "review").stdout,
            )
            run(local, "git", "push", "origin", ":refs/tags/review")
            self.assertEqual(
                run(local, *launcher, "--revision", "review", check=False).returncode, 2
            )
            del env["MUNDANE_TRY_DIR"]
            self.assertIn(
                "/mundane-try/qualification",
                run(local, *launcher, "--from-working-tree", "--headless").stdout,
            )
            self.assertIn(
                "/mundane-try/desktop",
                run(local, *launcher, "--from-working-tree").stdout,
            )
            env["MUNDANE_TRY_DIR"] = str(work / "cache/mundane-try/desktop")
            self.assertEqual(
                run(
                    local, *launcher, "--from-working-tree", "--headless", check=False
                ).returncode,
                2,
            )


if __name__ == "__main__":
    unittest.main()
