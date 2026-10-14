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

    def test_a_live_conflict_stops_before_a_build(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            _git_repo(repo, conflict=True)
            (root / "spread.json").write_text('{"frame_ms":{"spread":1}}\n', encoding="utf-8")
            out = root / "out"
            proc = run(
                "ab-verdict.py",
                [
                    "main",
                    "feature",
                    "--live",
                    "--display",
                    "wayland-1",
                    "--noise-floor",
                    str(root / "spread.json"),
                    "--repo",
                    str(repo),
                    "--out",
                    str(out),
                    "--cache-dir",
                    str(root / "cache"),
                ],
            )
            self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
            self.assertIn("merge conflict", proc.stdout)
            self.assertFalse((out / "build-base.log").exists())
            self.assertFalse((out / "build-head.log").exists())

    def test_a_cached_comparison_skips_the_build(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            _git_repo(repo, conflict=False)
            sha = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=repo,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            from lib.cache import cache_root, store
            from lib.contract import input_hash

            floor = (root / "spread.json").resolve()
            floor.write_text('{"frame_ms":{"spread":1}}\n', encoding="utf-8")
            digest = input_hash(sha, ["ab:main", "ab:feature", f"floor:{floor}", "repeats:"], [])
            cache = root / "cache"
            store(cache_root(str(cache)), sha, digest, "FAIL", "stress-still slower", root / "old")
            out = root / "out"
            proc = run(
                "ab-verdict.py",
                [
                    "main",
                    "feature",
                    "--live",
                    "--display",
                    "wayland-1",
                    "--noise-floor",
                    str(floor),
                    "--repo",
                    str(repo),
                    "--out",
                    str(out),
                    "--cache-dir",
                    str(cache),
                ],
            )
            self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
            self.assertIn("FAIL cached stress-still slower", proc.stdout)
            self.assertFalse((out / "build-base.log").exists())

    def test_omitted_noise_floor_is_measured_on_base_before_head(self) -> None:
        import importlib.util

        path = Path(__file__).resolve().parents[1] / "ab-verdict.py"
        spec = importlib.util.spec_from_file_location("ab_verdict_measure", path)
        assert spec is not None and spec.loader is not None
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            repo.mkdir()
            _git_repo(repo, conflict=True)
            proc = run(
                "ab-verdict.py",
                [
                    "main",
                    "feature",
                    "--live",
                    "--display",
                    "wayland-1",
                    "--repo",
                    str(repo),
                    "--out",
                    str(root / "conflict"),
                ],
            )
            self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
            self.assertIn("merge conflict", proc.stdout)
            self.assertNotIn("missing --noise-floor", proc.stdout)

            repo = root / "clean"
            repo.mkdir()
            _git_repo(repo, conflict=False)
            ns = mod.parse(
                ["main", "feature", "--live", "--display", "wayland-1", "--repo", str(repo)]
            )
            mod.validate(ns)
            self.assertEqual(mod.floor_job(ns), ("stress", "hall", 3, 180))
            order: list[str] = []

            def build(repo_path, sha, evidence, label, binary_name):
                order.append(label)
                binary = evidence / f"{label}-{binary_name}"
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_text("binary\n", encoding="utf-8")
                return binary

            def measure(parsed, binary, evidence):
                order.append(f"floor:{binary.name}")
                self.assertTrue(binary.name.startswith("base-"))
                floor = evidence / "noise-floor" / "spread.json"
                floor.parent.mkdir(parents=True, exist_ok=True)
                floor.write_text('{"frame_ms":{"spread":1}}\n', encoding="utf-8")
                return "FAIL", "base noise floor cold start did not settle within 180 frames", floor

            def capture_views(parsed, built, evidence):
                order.append("capture")
                return []

            out = root / "measured"
            status, reason, floor_path, rows = mod._live(
                ns,
                repo,
                out,
                build=build,
                measure=measure,
                capture_views=capture_views,
            )
            self.assertEqual(status, "FAIL")
            self.assertIn("base noise floor", reason)
            self.assertEqual(rows, [])
            self.assertEqual(order, ["base", "floor:base-genos-lighting"])
            self.assertTrue(floor_path.is_file())

            def measure_pass(parsed, binary, evidence):
                order.append(f"floor:{binary.name}")
                floor = evidence / "noise-floor" / "spread.json"
                floor.parent.mkdir(parents=True, exist_ok=True)
                floor.write_text('{"frame_ms":{"spread":1}}\n', encoding="utf-8")
                return "PASS", "base noise floor spread frame_ms=0.4", floor

            order.clear()
            status, _reason, floor_path, rows = mod._live(
                ns,
                repo,
                root / "passed-floor",
                build=build,
                measure=measure_pass,
                capture_views=capture_views,
            )
            self.assertEqual(status, "PASS")
            self.assertEqual(rows, [])
            self.assertEqual(order, ["base", "floor:base-genos-lighting", "head", "capture"])
            self.assertTrue(str(floor_path).endswith("noise-floor/spread.json"))


def _git(repo: Path, args: list[str]) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True)


def _git_repo(root: Path, conflict: bool) -> None:
    _git(root, ["init", "-b", "main"])
    _git(root, ["config", "user.email", "verdict@example.com"])
    _git(root, ["config", "user.name", "verdict"])
    (root / "file.txt").write_text("base\n", encoding="utf-8")
    _git(root, ["add", "file.txt"])
    _git(root, ["commit", "-m", "base"])
    _git(root, ["checkout", "-b", "feature"])
    (root / "file.txt").write_text("feature\n" if conflict else "base\nmore\n", encoding="utf-8")
    _git(root, ["add", "file.txt"])
    _git(root, ["commit", "-m", "feature"])
    if conflict:
        _git(root, ["checkout", "main"])
        (root / "file.txt").write_text("main\n", encoding="utf-8")
        _git(root, ["add", "file.txt"])
        _git(root, ["commit", "-m", "main"])
        _git(root, ["checkout", "feature"])


