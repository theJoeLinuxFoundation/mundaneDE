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
    run = Path(
        os.environ.get(
            "MUNDANE_TRY_DIR", cache / ("qualification" if args.headless else "desktop")
        )
    ).resolve()
    if args.headless and any(
        run.is_relative_to(cache / name) for name in ("desktop", "hand-test")
    ):
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

    def launch(root, label):
        subprocess.run(
            [
                sys.executable,
                Path(__file__).with_name("try_session.py"),
                "--source",
                root,
                "--build-label",
                label,
                *flags,
            ],
            env=env,
            check=True,
        )

    head = git("rev-parse", "HEAD")
    if args.from_working_tree:
        label = f"{head} (working tree, {'modified' if dirty else 'clean'})"
        print(f"Building mundaneDE {label}. Guest: {run}", flush=True)
        launch(ROOT, label)
        return

    # Refresh both namespaces so a stale branch or deleted remote tag cannot
    # make an unpublished commit eligible. Keep local tags independent.
    subprocess.run(
        [
            "git",
            "-C",
            ROOT,
            "fetch",
            "--prune",
            "--no-tags",
            "origin",
            "+refs/heads/*:refs/remotes/origin/*",
            "+refs/tags/*:refs/mundane-try/origin-tags/*",
        ],
        check=True,
    )

    def require_pushed(commit):
        refs = git(
            "for-each-ref",
            "--count=1",
            "--format=%(refname)",
            "--contains=" + commit,
            "refs/remotes/origin/",
            "refs/mundane-try/origin-tags/",
        )
        if not refs:
            parser.exit(
                2,
                f"{commit} has not been pushed to an origin branch or tag. Push it or explicitly pass --from-working-tree.\n",
            )

    require_pushed(head)
    revision = git(
        "rev-parse",
        "--verify",
        "--end-of-options",
        (args.revision or "refs/remotes/origin/main") + "^{commit}",
    )
    if revision != head:
        require_pushed(revision)
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
        launch(root, revision)


if __name__ == "__main__":
    main()
