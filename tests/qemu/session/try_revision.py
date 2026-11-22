"""Select and export the exact mundaneDE sources used for a hand test."""

import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[3]


def git(*args):
    return subprocess.check_output(["git", "-C", ROOT, *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "--from-working-tree",
        action="store_true",
        help="explicitly build the local checkout, including uncommitted changes",
    )
    source.add_argument(
        "--revision",
        help="build this committed revision (default: freshly fetched origin/main)",
    )
    parser.add_argument(
        "--headless",
        action="store_true",
        help="boot the qualification overlay without a window",
    )
    parser.add_argument(
        "--image", type=Path, help="frozen, read-only joe Linux test image"
    )
    args = parser.parse_args()
    dirty = bool(git("status", "--porcelain", "--untracked-files=normal"))
    if dirty and not args.from_working_tree:
        parser.exit(
            2,
            "The checkout contains uncommitted changes. Run from a clean checkout or explicitly pass --from-working-tree.\n",
        )
    cache = (
        Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")).resolve()
        / "mundane-try"
    )
    desktop = (cache / "desktop").resolve()
    run = Path(
        os.environ.get(
            "MUNDANE_TRY_DIR", cache / ("qualification" if args.headless else "desktop")
        )
    ).resolve()
    if args.headless and (run == desktop or desktop in run.parents):
        parser.exit(
            2,
            "Headless qualification needs its own MUNDANE_TRY_DIR outside the desktop hand-test directory.\n",
        )
    env = {**os.environ, "MUNDANE_TRY_DIR": str(run)}
    # Each workspace has its own target directory in this exported build.
    env.pop("CARGO_TARGET_DIR", None)
    flags = ["--headless"] if args.headless else []
    if args.image:
        flags.extend(["--image", str(args.image.resolve())])

    def launch(root):
        subprocess.run(
            [sys.executable, root / "tests/qemu/session/try_session.py", *flags],
            env=env,
            check=True,
        )

    if args.from_working_tree:
        print(
            f"Building working tree at {git('rev-parse', 'HEAD')} ({'modified' if dirty else 'clean'}). Guest: {run}",
            flush=True,
        )
        launch(ROOT)
        return
    if not args.revision:
        subprocess.run(
            [
                "git",
                "-C",
                ROOT,
                "fetch",
                "--no-tags",
                "origin",
                "refs/heads/main:refs/remotes/origin/main",
            ],
            check=True,
        )
    revision = git(
        "rev-parse",
        "--verify",
        "--end-of-options",
        (args.revision or "refs/remotes/origin/main") + "^{commit}",
    )
    print(f"Building committed mundaneDE {revision}. Guest: {run}", flush=True)
    builds = cache / "build" / hashlib.sha256(os.fsencode(run)).hexdigest()[:20]
    builds.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=revision[:12] + "-", dir=builds) as work:
        root = Path(work)
        with subprocess.Popen(
            ["git", "-C", ROOT, "archive", "--format=tar", revision],
            stdout=subprocess.PIPE,
        ) as proc:
            with tarfile.open(fileobj=proc.stdout, mode="r|") as archive:
                archive.extractall(root, filter="data")
            if proc.wait():
                raise RuntimeError("git archive failed")
        # Cache only Cargo output. Every invocation gets freshly exported source.
        for workspace, name in [
            (root, "compositor"),
            (root / "settings-panel", "settings"),
        ]:
            target = builds / ("target-" + name)
            target.mkdir(exist_ok=True)
            (workspace / "target").symlink_to(target, target_is_directory=True)
        launch(root)


if __name__ == "__main__":
    main()
