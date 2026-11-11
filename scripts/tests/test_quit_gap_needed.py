#!/usr/bin/env python3
"""The quit-gap filter runs the job for anything the windowed binaries compile."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci-quit-gap-needed.sh"


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
        cwd=repo,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


class QuitGapFilter(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory()
        repo = Path(cls.tmp.name)
        git(repo, "init", "-q")
        manifests = [p for p in ROOT.rglob("Cargo.toml") if "target" not in p.parts]
        for manifest in manifests:
            dest = repo / manifest.relative_to(ROOT)
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(manifest.read_bytes())
        git(repo, "add", "-A")
        git(repo, "commit", "-qm", "base")
        cls.repo = repo
        cls.base = git(repo, "rev-parse", "HEAD")

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def needed(self, path: str) -> bool:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("a") as handle:
            handle.write("x\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "change")
        head = git(self.repo, "rev-parse", "HEAD")
        try:
            result = subprocess.run(
                ["bash", str(SCRIPT), self.base, head],
                cwd=self.repo,
                capture_output=True,
                text=True,
                check=False,
            )
        finally:
            git(self.repo, "reset", "-q", "--hard", self.base)
            git(self.repo, "clean", "-qfd")
        self.assertIn(result.returncode, (0, 1), result.stderr)
        return result.returncode == 0

    def test_lifecycle_window_and_quit_code_runs_the_job(self) -> None:
        for path in (
            "crates/window/src/lifecycle.rs",
            "crates/render/src/gpu.rs",
            "crates/render/src/anything_else.rs",
            "crates/scene/src/event_loop.rs",
            "crates/mcp/src/lib.rs",
            "crates/input/src/lib.rs",
            "examples/stress/src/main.rs",
            "debug/lighting/src/main.rs",
            "Cargo.lock",
            ".github/workflows/lint.yml",
        ):
            with self.subTest(path=path):
                self.assertTrue(self.needed(path))

    def test_an_unknown_path_runs_the_job(self) -> None:
        self.assertTrue(self.needed("somewhere/new.txt"))

    def test_docs_and_crates_outside_the_closure_skip_the_job(self) -> None:
        for path in (
            "docs/systems/lighting.md",
            "README.md",
            "crates/jobs/src/pool.rs",
            "crates/ecs/src/world.rs",
            "debug/particles/src/main.rs",
        ):
            with self.subTest(path=path):
                self.assertFalse(self.needed(path))


if __name__ == "__main__":
    unittest.main()
