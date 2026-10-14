"""Fixed-pose image diff."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from lib.images import solid, write_png


def run(script: str, argv: list[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    for name in list(env):
        if name.startswith("GENOS_VERDICT_"):
            env.pop(name)
    return subprocess.run(
        [sys.executable, str(Path(__file__).resolve().parents[1] / script), *argv],
        check=False,
        text=True,
        capture_output=True,
        env=env,
    )


class PoseTests(unittest.TestCase):
    def test_matching_pose_passes_and_records_ui_false(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_png(root / "before.png", solid(8, 4, (0.2, 0.2, 0.2)))
            write_png(root / "after.png", solid(8, 4, (0.2, 0.2, 0.2)))
            out = root / "out"
            proc = run(
                "pose-diff.py",
                ["--fixture", str(root), "--out", str(out), "--sha", "hall"],
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertTrue(proc.stdout.startswith("PASS "))
            self.assertIn('"ui": false', (out / "capture.json").read_text(encoding="utf-8"))
            self.assertTrue((out / "diff.png").is_file())

    def test_different_pose_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_png(root / "before.png", solid(8, 4, (1.0, 0.0, 0.0)))
            write_png(root / "after.png", solid(8, 4, (0.0, 0.0, 1.0)))
            proc = run("pose-diff.py", ["--fixture", str(root), "--out", str(root / "out"), "--sha", "hall"])
            self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
            self.assertTrue(proc.stdout.startswith("FAIL "))

    def test_first_pose_is_unsure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_png(root / "after.png", solid(4, 2, (0.2, 0.2, 0.2)))
            proc = run("pose-diff.py", ["--fixture", str(root), "--out", str(root / "out"), "--sha", "hall"])
            self.assertEqual(proc.returncode, 2, proc.stdout)
            self.assertIn("no previous pose", proc.stdout)


