"""Launch argv against the real genos-lighting binaries, with no GPU."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.catalog import available
from lib.launch import binary_help, child_env, help_flags, lighting_launch
from lighting_bin import lighting_binary


# The two commits a live A/B actually built: gi-v2, and the head from the GPU run.
REVISIONS = ("55795f6", "8a7e849398cc91c4c635c53f131b73926e3fb9d1")


def _parse(binary: Path, argv: list[str], extra: dict[str, str]) -> subprocess.CompletedProcess[str]:
    env = child_env({"PATH": os.environ.get("PATH", "")}, "wayland-1", extra)
    env.pop("XDG_RUNTIME_DIR", None)
    return subprocess.run(
        argv,
        check=False,
        text=True,
        capture_output=True,
        timeout=20,
        env=env,
    )


class RealArgvTests(unittest.TestCase):
    def test_each_live_script_argv_parses_on_the_real_binaries(self) -> None:
        """noise-floor and ab-verdict both launch through lighting_launch."""
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "shots"
            for rev in REVISIONS:
                binary = lighting_binary(rev)
                text = binary_help(str(binary))
                allowed = help_flags(text)
                plans = [
                    ("noise-floor", "stress", "hall"),
                    ("ab-floor", "stress", "hall"),
                ]
                plans.extend(
                    (f"ab:{view.scene}/{view.checkpoint}", view.lighting_scene, view.lighting_checkpoint)
                    for view in available()
                )
                for label, scene, checkpoint in plans:
                    argv, extra = lighting_launch(str(binary), "wayland-1", scene, checkpoint, 9, out)
                    flags = [token for token in argv[1:] if token.startswith("--")]
                    for flag in flags:
                        self.assertIn(flag, allowed, f"{rev} {label} {argv}")
                    self.assertNotIn("--display", argv, label)
                    self.assertNotIn("--ui", argv, label)
                    self.assertNotIn("GENOS_MCP_PORT", extra, label)
                    proc = _parse(binary, argv, extra)
                    blob = (proc.stdout or "") + (proc.stderr or "")
                    self.assertNotIn("unknown argument", blob, f"{rev} {label}\n{blob}")
                gi_v2 = rev.startswith("55795f6")
                argv, extra = lighting_launch(str(binary), "wayland-1", "stress", "hall", 9, out)
                if gi_v2:
                    self.assertEqual(
                        argv[1:],
                        [
                            "--headless",
                            "--scene",
                            "stress",
                            "--checkpoint",
                            "hall",
                            "--mcp-port",
                            "9",
                            "--out",
                            str(out.resolve()),
                            "--size",
                            "320x180",
                            "--menu",
                            "hidden",
                        ],
                    )
                    self.assertEqual(extra, {})
