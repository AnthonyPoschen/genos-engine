"""Noise floor, picture diff, and the rule that a window is stopped without SIGKILL."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from lib.contract import Broken, capture_file
from lib.images import flip_mean_p99, load_image, solid, write_png
from lib.launch import child_env, help_lists_display, lighting_argv, lighting_launch, stop, stress_argv
from lib.noise import summarize


def run_script(argv: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    for name in list(env):
        if name.startswith("GENOS_VERDICT_"):
            env.pop(name)
    return subprocess.run(
        [sys.executable, str(Path(__file__).resolve().parents[1] / "noise-floor.py"), *argv],
        check=False,
        text=True,
        capture_output=True,
        env=env,
        cwd=cwd,
    )


class NoiseTests(unittest.TestCase):
    def test_dry_run_requires_a_display(self) -> None:
        proc = run_script(["--dry-run", "--repeats", "3", "--base", "gi-v2", "--scene", "stress", "--checkpoint", "hall"])
        self.assertEqual(proc.returncode, 3, proc.stdout)
        self.assertTrue(proc.stdout.startswith("ERROR missing --display "))
        self.assertNotIn("UNSURE", proc.stdout)
        self.assertEqual(len(proc.stdout.splitlines()), 1)

    def test_dry_run_writes_capture_ui_false(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            proc = run_script(
                [
                    "--dry-run",
                    "--repeats",
                    "3",
                    "--base",
                    "gi-v2",
                    "--scene",
                    "stress",
                    "--checkpoint",
                    "hall",
                    "--display",
                    "wayland-9",
                    "--out",
                    str(out),
                ]
            )
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            self.assertTrue(proc.stdout.startswith("PASS dry-run evidence="))
            self.assertIn('"ui": false', (out / "capture.json").read_text(encoding="utf-8"))

    def test_cold_start_past_180_frames_fails(self) -> None:
        rows = [
            {"repeat": "1", "frame_ms": "16", "settled": "1", "settle_frames": "40", "image": ""},
            {"repeat": "2", "frame_ms": "17", "settled": "0", "settle_frames": "180", "image": ""},
        ]
        status, reason, payload = summarize(rows, 180)
        self.assertEqual(status, "FAIL")
        self.assertIn("180", reason)
        self.assertEqual(payload["frame_ms"]["spread"], 1)

    def test_settled_repeats_pass_and_store_the_spread(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = solid(8, 4, (0.4, 0.35, 0.2))
            write_png(root / "a.png", image)
            write_png(root / "b.png", image)
            rows = "\n".join(
                [
                    "repeat,frame_ms,settled,settle_frames,image",
                    "1,16.0,1,30,a.png",
                    "2,16.4,1,36,b.png",
                    "",
                ]
            )
            (root / "repeats.csv").write_text(rows, encoding="utf-8")
            out = root / "out"
            proc = run_script(
                ["--fixture", str(root), "--repeats", "2", "--out", str(out), "--sha", "abc123"]
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertIn("PASS spread frame_ms=", proc.stdout)
            self.assertEqual(len(proc.stdout.splitlines()), 1)
            self.assertTrue((out / "spread.json").is_file())
            self.assertTrue((out / "numbers.csv").is_file())
            spread = (out / "spread.json").read_text(encoding="utf-8")
            self.assertIn('"spread": 0.0', spread)

    def test_one_sample_is_unsure(self) -> None:
        rows = [{"repeat": "1", "frame_ms": "16", "settled": "1", "settle_frames": "10", "image": ""}]
        status, reason, _payload = summarize(rows, 180)
        self.assertEqual(status, "UNSURE")
        self.assertEqual(reason, "one sample")

    def test_identical_pictures_have_no_flip_error(self) -> None:
        picture = solid(16, 8, (0.4, 0.35, 0.2))
        mean, p99 = flip_mean_p99(picture, picture)
        self.assertLess(mean, 1.0e-5)
        self.assertLess(p99, 1.0e-5)

    def test_a_brightness_bias_raises_flip(self) -> None:
        reference = solid(16, 8, (0.4, 0.4, 0.4))
        candidate = solid(16, 8, (0.56, 0.56, 0.56))
        mean, _p99 = flip_mean_p99(reference, candidate)
        self.assertGreater(mean, 0.2)

    def test_png_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "shot.png"
            write_png(path, solid(4, 2, (1.0, 0.0, 0.0)))
            loaded = load_image(path)
            self.assertEqual(loaded.width, 4)
            self.assertEqual(loaded.at(0, 0), (1.0, 0.0, 0.0))

    def test_relative_out_from_another_cwd_is_absolute(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            caller = Path(tmp) / "caller"
            caller.mkdir()
            fixture = Path(tmp) / "fixture"
            fixture.mkdir()
            (fixture / "repeats.csv").write_text(
                "repeat,frame_ms,settled,settle_frames,image\n"
                "1,16.0,1,30,\n"
                "2,16.4,1,36,\n",
                encoding="utf-8",
            )
            previous = Path.cwd()
            try:
                os.chdir(caller)
                shot = capture_file(Path("rel/out") / "run-1")
                argv = lighting_argv(
                    str(
                        _help_binary(
                            caller / "lighting",
                            "--display NAME --headless --scene NAME --checkpoint NAME --mcp-port N --out DIR\n",
                        )
                    ),
                    "wayland-9",
                    "stress",
                    "hall",
                    9,
                    Path("rel/out/run-1"),
                )
            finally:
                os.chdir(previous)
            self.assertTrue(shot.is_absolute())
            self.assertEqual(shot, (caller / "rel/out/run-1/pose.png").resolve())
            self.assertIn(str((caller / "rel/out/run-1").resolve()), argv)
            proc = run_script(
                [
                    "--fixture",
                    str(fixture),
                    "--repeats",
                    "2",
                    "--sha",
                    "abc123",
                    "--out",
                    "rel/out",
                ],
                cwd=caller,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            evidence = (caller / "rel/out").resolve()
            self.assertIn(f"evidence={evidence}", proc.stdout)
            self.assertTrue(Path(proc.stdout.strip().split("evidence=", 1)[1]).is_absolute())
            self.assertTrue((evidence / "spread.json").is_file())
            self.assertFalse((previous / "rel/out/spread.json").exists())

    def test_launch_targets_the_named_display_and_not_sigkill(self) -> None:
        self.assertTrue(help_lists_display("  --display NAME         Wayland display name\n"))
        self.assertFalse(help_lists_display("  --headless             proof window, no panel\n"))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            new = _help_binary(
                root / "new-lighting",
                "--headless --display NAME --scene NAME --checkpoint NAME --mcp-port N --out DIR\n",
            )
            old = _help_binary(
                root / "old-lighting",
                "--headless proof window --scene NAME --checkpoint NAME --out DIR GENOS_MCP_PORT\n",
            )
            argv = lighting_argv(str(new), "wayland-9", "stress", "hall", 9, Path("out"))
            self.assertEqual(argv[1:3], ["--headless", "--display"])
            self.assertEqual(argv[3], "wayland-9")
            self.assertIn("--mcp-port", argv)
            self.assertNotIn("HDMI-A-1", argv)
            self.assertNotIn("DP-1", argv)
            old_argv, old_env = lighting_launch(str(old), "wayland-9", "stress", "hall", 9, Path("out"))
            self.assertNotIn("--display", old_argv)
            self.assertNotIn("--mcp-port", old_argv)
            self.assertNotIn("wayland-9", old_argv)
            self.assertIn("--headless", old_argv)
            self.assertEqual(old_env["GENOS_MCP_PORT"], "9")
            bare = _help_binary(
                root / "bare-lighting",
                "--headless --scene NAME --checkpoint NAME\n",
            )
            bare_argv, bare_env = lighting_launch(str(bare), "wayland-9", "stress", "hall", 9, Path("out"))
            self.assertNotIn("--mcp-port", bare_argv)
            self.assertEqual(bare_env.get("GENOS_MCP_PORT"), "9")
            old_stress = stress_argv(str(old), "wayland-9", 30, Path("out"))
            self.assertNotIn("--display", old_stress)
            self.assertIn("--proof", old_stress)
        env = child_env({"WAYLAND_DISPLAY": "wayland-0", "DISPLAY": ":0", "PATH": "/bin"}, "wayland-9")
        self.assertEqual(env["WAYLAND_DISPLAY"], "wayland-9")
        self.assertNotIn("DISPLAY", env)
        root = Path(__file__).resolve().parents[1]
        for path in root.rglob("*.py"):
            if "tests" in path.parts:
                continue
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("SIGKILL", text, path)
            self.assertNotIn("proc.kill", text, path)
            self.assertNotIn("kill -9", text, path)

    def test_stop_uses_sigterm_and_does_not_kill(self) -> None:
        proc = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(30)",
            ]
        )
        try:
            with self.assertRaises(Broken):
                stop(proc, lambda: None, 0.3)
            self.assertIsNone(proc.poll())
        finally:
            proc.kill()
            proc.wait(timeout=2)


def _help_binary(path: Path, text: str) -> Path:
    path.write_text(f"#!/bin/sh\nprintf '%s' {text!r}\n", encoding="utf-8")
    path.chmod(0o755)
    return path


if __name__ == "__main__":
    unittest.main()
