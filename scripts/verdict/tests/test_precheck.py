"""Precheck: merge-tree, GPU busy, and the verdict cache. No GPU."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from lib.cache import cache_root, store
from lib.contract import CODE, FAIL, PASS, UNSURE
from lib.gpu import classify
import precheck


def run_script(argv: list[str], env: dict | None = None) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    for name in (
        "GENOS_VERDICT_OUT",
        "GENOS_VERDICT_DISPLAY",
        "GENOS_VERDICT_BASE",
        "GENOS_VERDICT_HEAD",
        "GENOS_VERDICT_SHA",
        "GENOS_VERDICT_CACHE_DIR",
        "GENOS_VERDICT_GPU_BUSY_PERCENT",
    ):
        merged.pop(name, None)
    if env:
        merged.update(env)
    return subprocess.run(
        [sys.executable, str(Path(__file__).resolve().parents[1] / "precheck.py"), *argv],
        check=False,
        text=True,
        capture_output=True,
        env=merged,
    )


def git(repo: Path, args: list[str]) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True)


def init_repo(root: Path, conflict: bool) -> tuple[str, str]:
    git(root, ["init", "-b", "main"])
    git(root, ["config", "user.email", "verdict@example.com"])
    git(root, ["config", "user.name", "verdict"])
    (root / "file.txt").write_text("base\n", encoding="utf-8")
    git(root, ["add", "file.txt"])
    git(root, ["commit", "-m", "base"])
    base = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True
    ).stdout.strip()
    git(root, ["checkout", "-b", "feature"])
    (root / "file.txt").write_text("feature\n" if conflict else "base\nmore\n", encoding="utf-8")
    if not conflict:
        (root / "other.txt").write_text("feature\n", encoding="utf-8")
        git(root, ["add", "other.txt"])
    git(root, ["add", "file.txt"])
    git(root, ["commit", "-m", "feature"])
    if conflict:
        git(root, ["checkout", "main"])
        (root / "file.txt").write_text("main\n", encoding="utf-8")
        git(root, ["add", "file.txt"])
        git(root, ["commit", "-m", "main"])
        git(root, ["checkout", "feature"])
    return base, "feature"


class PrecheckTests(unittest.TestCase):
    def test_dry_run_is_one_pass_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            proc = run_script(["--dry-run", "--base", "gi-v2", "--sha", "abc", "--out", str(out)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        lines = [line for line in proc.stdout.splitlines() if line]
        self.assertEqual(len(lines), 1)
        self.assertTrue(lines[0].startswith("PASS dry-run evidence="))
        self.assertIn(str(out), lines[0])

    def test_missing_base_is_a_broken_check(self) -> None:
        proc = run_script(["--dry-run", "--out", "genos-debug/verdict/precheck/unknown"])
        self.assertEqual(proc.returncode, 3, proc.stdout)
        self.assertEqual(len(proc.stdout.splitlines()), 1)
        self.assertTrue(proc.stdout.startswith("ERROR missing --base "))

    def test_flag_wins_over_the_env_mirror(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "from-flag"
            proc = run_script(
                ["--dry-run", "--base", "gi-v2", "--sha", "abc", "--out", str(out)],
                env={"GENOS_VERDICT_OUT": str(Path(tmp) / "from-env"), "GENOS_VERDICT_BASE": "other"},
            )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(str(out), proc.stdout)
        self.assertNotIn("from-env", proc.stdout)

    def test_env_mirror_fills_an_omitted_flag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "from-env"
            proc = run_script(
                ["--dry-run", "--sha", "abc"],
                env={"GENOS_VERDICT_OUT": str(out), "GENOS_VERDICT_BASE": "gi-v2"},
            )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(str(out), proc.stdout)

    def test_clean_tree_and_idle_gpu_pass(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _base, head = init_repo(root, conflict=False)
            out = Path(tmp) / "out"
            processes = Path(tmp) / "ps.txt"
            processes.write_text("python3 precheck.py\n", encoding="utf-8")
            util = Path(tmp) / "gpu.txt"
            util.write_text("12\n", encoding="utf-8")
            proc = run_script(
                [
                    "--base",
                    "main",
                    "--head",
                    head,
                    "--repo",
                    str(root),
                    "--out",
                    str(out),
                    "--process-file",
                    str(processes),
                    "--gpu-util-file",
                    str(util),
                    "--lock",
                    str(Path(tmp) / "missing.lock"),
                    "--cache-dir",
                    str(Path(tmp) / "cache"),
                ]
            )
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            self.assertTrue(proc.stdout.startswith("PASS clear gpu 12% evidence="))
            self.assertIn(str(out), proc.stdout)
            self.assertTrue((out / "merge-tree.log").is_file())
            self.assertTrue((out / "gpu.txt").is_file())

    def test_merge_conflict_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            init_repo(root, conflict=True)
            out = Path(tmp) / "out"
            proc = run_script(
                [
                    "--base",
                    "main",
                    "--repo",
                    str(root),
                    "--out",
                    str(out),
                    "--process-file",
                    str(Path(tmp) / "ps.txt"),
                    "--gpu-util-file",
                    str(Path(tmp) / "gpu.txt"),
                ]
            )
            self.assertEqual(proc.returncode, CODE[FAIL], proc.stdout + proc.stderr)
            self.assertIn("FAIL merge conflict with main evidence=", proc.stdout)
            self.assertIn("file.txt", (out / "merge-tree.log").read_text(encoding="utf-8"))

    def test_busy_gpu_is_unsure_and_does_not_need_a_window(self) -> None:
        self.assertEqual(classify(["python"], 80, 50, None), (UNSURE, "busy gpu 80%"))
        self.assertEqual(
            classify(["/usr/local/bin/genos-stress --proof"], 10, 50, None),
            (UNSURE, "busy genos-stress"),
        )
        self.assertEqual(classify(["python"], 10, 50, 42), (UNSURE, "busy lock pid=42"))

    def test_cache_hit_replays_the_verdict(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            root.mkdir()
            _base, _head = init_repo(root, conflict=False)
            sha = subprocess.run(
                ["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True
            ).stdout.strip()
            from lib.contract import input_hash

            digest = input_hash(sha, ["noise-floor"], [])
            cache = Path(tmp) / "cache"
            store(cache_root(str(cache)), sha, digest, FAIL, "stress-still slower", Path(tmp) / "old")
            out = Path(tmp) / "out"
            processes = Path(tmp) / "ps.txt"
            processes.write_text("idle\n", encoding="utf-8")
            util = Path(tmp) / "gpu.txt"
            util.write_text("1\n", encoding="utf-8")
            proc = run_script(
                [
                    "--base",
                    "main",
                    "--repo",
                    str(root),
                    "--sha",
                    sha,
                    "--key",
                    "noise-floor",
                    "--out",
                    str(out),
                    "--cache-dir",
                    str(cache),
                    "--process-file",
                    str(processes),
                    "--gpu-util-file",
                    str(util),
                    "--lock",
                    str(Path(tmp) / "no.lock"),
                ]
            )
            self.assertEqual(proc.returncode, CODE[FAIL], proc.stdout)
            self.assertIn("FAIL cached stress-still slower evidence=", proc.stdout)
            saved = json.loads((out / "cache.json").read_text(encoding="utf-8"))
            self.assertEqual(saved["status"], FAIL)
            self.assertEqual(len(proc.stdout.splitlines()), 1)

    def test_display_connector_names_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            proc = run_script(
                [
                    "--dry-run",
                    "--base",
                    "gi-v2",
                    "--sha",
                    "abc",
                    "--display",
                    "DP-1",
                    "--out",
                    str(Path(tmp) / "out"),
                ]
            )
        self.assertEqual(proc.returncode, 3)
        self.assertTrue(proc.stdout.startswith("ERROR "))
        self.assertNotIn("UNSURE", proc.stdout)


if __name__ == "__main__":
    unittest.main()
