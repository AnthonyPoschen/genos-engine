"""PR head watcher."""

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


class WatchTests(unittest.TestCase):
    def test_only_new_heads_wake_the_watcher(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            prs = root / "prs.json"
            prs.write_text(
                json.dumps([{"number": 7, "head": "aaa", "url": "http://example/7"}]),
                encoding="utf-8",
            )
            state = root / "state.json"
            out = root / "out"
            first = run("pr-watch.py", ["--prs-file", str(prs), "--state", str(state), "--out", str(out)])
            self.assertEqual(first.returncode, 0, first.stdout)
            self.assertIn("baseline", first.stdout)
            self.assertEqual((out / "new-heads.txt").read_text(encoding="utf-8"), "")
            second = run(
                "pr-watch.py",
                ["--prs-file", str(prs), "--state", str(state), "--out", str(root / "out2")],
            )
            self.assertEqual(second.returncode, 0, second.stdout)
            self.assertIn("no new heads", second.stdout)
            prs.write_text(
                json.dumps(
                    [
                        {"number": 7, "head": "bbb", "url": "http://example/7"},
                        {"number": 8, "head": "ccc", "url": "http://example/8"},
                    ]
                ),
                encoding="utf-8",
            )
            out3 = root / "out3"
            third = run("pr-watch.py", ["--prs-file", str(prs), "--state", str(state), "--out", str(out3)])
            self.assertEqual(third.returncode, 2, third.stdout)
            self.assertIn("new heads count=2", third.stdout)
            heads = (out3 / "new-heads.txt").read_text(encoding="utf-8")
            self.assertIn("7 bbb", heads)
            self.assertIn("8 ccc", heads)
            self.assertEqual(len(third.stdout.splitlines()), 1)


