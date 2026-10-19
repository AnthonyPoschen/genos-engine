"""Checkpoint catalog blur and FLIP."""

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


class CheckpointTests(unittest.TestCase):
    def test_band_and_no_reference_lock(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "repeats.csv").write_text(
                "scene,checkpoint,repeat,blur_rel,flip_mean\n"
                "stress-still,hall,1,0.01,0.02\n"
                "stress-still,hall,2,0.02,0.03\n"
                "bounce-corner,corner,1,0.12,0.10\n"
                "bounce-corner,corner,2,0.12,0.10\n"
                "hell-100,night-hall,1,0.30,0.10\n"
                "hell-100,night-hall,2,0.30,0.10\n",
                encoding="utf-8",
            )
            out = root / "out"
            proc = run(
                "checkpoint-catalog.py",
                ["--fixture", str(root), "--repeats", "2", "--out", str(out), "--sha", "abc"],
            )
            self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
            self.assertIn("FAIL", proc.stdout)
            saved = (out / "checkpoints.csv").read_text(encoding="utf-8")
            self.assertIn("stress-still,hall,PASS", saved)
            self.assertIn("bounce-corner,corner,UNSURE", saved)
            self.assertIn("hell-100,night-hall,FAIL", saved)
            self.assertIn("blur_spread", saved)
            self.assertFalse((out / "reference").exists())
            source = Path(__file__).resolve().parents[1].joinpath("checkpoint-catalog.py").read_text(encoding="utf-8")
            self.assertNotIn("genos-debug/reference", source)

    def test_one_repeat_is_unsure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "repeats.csv").write_text(
                "scene,checkpoint,repeat,blur_rel,flip_mean\nstress-still,hall,1,0.01,0.01\n",
                encoding="utf-8",
            )
            proc = run(
                "checkpoint-catalog.py",
                ["--fixture", str(root), "--out", str(root / "out"), "--sha", "abc"],
            )
            self.assertEqual(proc.returncode, 2, proc.stdout)
            self.assertIn("one sample", proc.stdout)


