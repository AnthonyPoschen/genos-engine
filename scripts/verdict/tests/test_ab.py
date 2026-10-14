"""A/B verdict against the noise floor."""

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


class AbTests(unittest.TestCase):
    def test_within_noise_passes_and_writes_scene_verdicts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "spread.json").write_text(
                json.dumps(
                    {
                        "frame_ms": {"spread": 1.0},
                        "mean_abs": {"spread": 0.05},
                        "flip_mean": {"spread": 0.2},
                        "blur_rel": {"spread": 0.05},
                    }
                ),
                encoding="utf-8",
            )
            (root / "views.csv").write_text(
                "scene,checkpoint,base_ms,head_ms,mean_abs,flip_mean,blur_rel\n"
                "stress-still,hall,16,16.2,0.01,0.02,0.01\n"
                "bounce-corner,corner,20,20.4,0.01,0.02,0.01\n",
                encoding="utf-8",
            )
            out = root / "out"
            proc = run(
                "ab-verdict.py",
                ["aaa", "bbb", "--fixture", str(root), "--out", str(out), "--sha", "bbb"],
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertIn("PASS", proc.stdout)
            self.assertEqual(len(proc.stdout.splitlines()), 1)
            text = (out / "scenes.csv").read_text(encoding="utf-8")
            self.assertIn("stress-still,PASS", text)
            self.assertIn("bounce-corner,PASS", text)

    def test_slower_than_noise_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "spread.json").write_text(
                json.dumps({"frame_ms": {"spread": 0.5}, "mean_abs": {"spread": 0.05}, "flip_mean": {"spread": 0.2}, "blur_rel": {"spread": 0.05}}),
                encoding="utf-8",
            )
            (root / "views.csv").write_text(
                "scene,checkpoint,base_ms,head_ms,mean_abs,flip_mean,blur_rel\n"
                "stress-still,hall,16,20,0.01,0.02,0.01\n",
                encoding="utf-8",
            )
            proc = run(
                "ab-verdict.py",
                ["aaa", "bbb", "--fixture", str(root), "--out", str(root / "out"), "--sha", "bbb"],
            )
            self.assertEqual(proc.returncode, 1, proc.stdout)
            self.assertIn("FAIL stress-still/hall", proc.stdout)

    def test_borderline_frame_is_unsure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "spread.json").write_text(
                json.dumps({"frame_ms": {"spread": 1.0}, "mean_abs": {"spread": 0.05}, "flip_mean": {"spread": 0.2}, "blur_rel": {"spread": 0.05}}),
                encoding="utf-8",
            )
            # delta 1.5 is above the spread and below 2x.
            (root / "views.csv").write_text(
                "scene,checkpoint,base_ms,head_ms,mean_abs,flip_mean,blur_rel\n"
                "hell-100,night-hall,10,11.5,0.01,0.02,0.01\n",
                encoding="utf-8",
            )
            proc = run(
                "ab-verdict.py",
                ["aaa", "bbb", "--fixture", str(root), "--out", str(root / "out"), "--sha", "bbb"],
            )
            self.assertEqual(proc.returncode, 2, proc.stdout)
            self.assertTrue(proc.stdout.startswith("UNSURE "))


