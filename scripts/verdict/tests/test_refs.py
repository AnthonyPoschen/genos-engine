"""Branch names resolve to origin/<name> after git fetch. A stale local branch is not built."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def run_precheck(argv: list[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    for name in list(env):
        if name.startswith("GENOS_VERDICT_"):
            env.pop(name)
    return subprocess.run(
        [sys.executable, str(Path(__file__).resolve().parents[1] / "precheck.py"), *argv],
        check=False,
        text=True,
        capture_output=True,
        env=env,
    )


def git(repo: Path, args: list[str]) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True)


def rev(repo: Path, ref: str) -> str:
    proc = subprocess.run(
        ["git", "rev-parse", ref],
        cwd=repo,
        check=True,
        capture_output=True,
        text=True,
    )
    return proc.stdout.strip()


def init_pair(tmp: Path) -> tuple[Path, Path, str]:
    """Remote gi-v2 at OLD, cloned, then the remote advanced. The clone is not pulled."""
    remote = tmp / "remote"
    remote.mkdir()
    git(remote, ["init", "-b", "gi-v2"])
    git(remote, ["config", "user.email", "verdict@example.com"])
    git(remote, ["config", "user.name", "verdict"])
    (remote / "file.txt").write_text("old\n", encoding="utf-8")
    git(remote, ["add", "file.txt"])
    git(remote, ["commit", "-m", "old"])
    clone = tmp / "clone"
    subprocess.run(
        ["git", "clone", "--origin", "origin", str(remote), str(clone)],
        check=True,
        capture_output=True,
        text=True,
    )
    old = rev(clone, "gi-v2")
    (remote / "file.txt").write_text("new\n", encoding="utf-8")
    git(remote, ["add", "file.txt"])
    git(remote, ["commit", "-m", "new"])
    return remote, clone, old


class StaleBranchTests(unittest.TestCase):
    def test_a_stale_local_branch_resolves_to_origin(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            remote, clone, old = init_pair(tmp)
            new = rev(remote, "HEAD")
            self.assertNotEqual(old, new)
            self.assertEqual(rev(clone, "refs/heads/gi-v2"), old)
            self.assertEqual(rev(clone, "refs/remotes/origin/gi-v2"), old)

            dry = run_precheck(
                [
                    "--dry-run",
                    "--base",
                    "gi-v2",
                    "--sha",
                    "label",
                    "--repo",
                    str(clone),
                    "--out",
                    str(tmp / "dry"),
                ]
            )
            self.assertEqual(dry.returncode, 0, dry.stderr)
            self.assertEqual(rev(clone, "refs/remotes/origin/gi-v2"), old)

            processes = tmp / "ps.txt"
            processes.write_text("python3 precheck.py\n", encoding="utf-8")
            util = tmp / "gpu.txt"
            util.write_text("12\n", encoding="utf-8")
            out = tmp / "out"
            proc = run_precheck(
                [
                    "--base",
                    "gi-v2",
                    "--repo",
                    str(clone),
                    "--out",
                    str(out),
                    "--process-file",
                    str(processes),
                    "--gpu-util-file",
                    str(util),
                    "--lock",
                    str(tmp / "missing.lock"),
                    "--cache-dir",
                    str(tmp / "cache"),
                ]
            )
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            self.assertEqual(len(proc.stdout.splitlines()), 1)
            self.assertIn(f"base={new}", proc.stdout)
            self.assertIn(f"head={old}", proc.stdout)
            self.assertNotIn(f"base={old}", proc.stdout)
            saved = (out / "sha.txt").read_text(encoding="utf-8")
            self.assertIn(f"base {new}\n", saved)
            self.assertIn(f"head {old}\n", saved)
            self.assertNotIn(f"base {old}", saved)
            self.assertEqual(rev(clone, "refs/heads/gi-v2"), old)
            self.assertEqual(rev(clone, "refs/remotes/origin/gi-v2"), new)

            refused = run_precheck(
                [
                    "--base",
                    "refs/heads/gi-v2",
                    "--repo",
                    str(clone),
                    "--out",
                    str(tmp / "local"),
                    "--process-file",
                    str(processes),
                    "--gpu-util-file",
                    str(util),
                    "--lock",
                    str(tmp / "missing.lock"),
                ]
            )
            self.assertEqual(refused.returncode, 3, refused.stdout + refused.stderr)
            self.assertTrue(refused.stdout.startswith("ERROR gi-v2 is behind origin/gi-v2 "))
            self.assertEqual(len(refused.stdout.splitlines()), 1)
            self.assertEqual(rev(clone, "refs/heads/gi-v2"), old)


if __name__ == "__main__":
    unittest.main()
