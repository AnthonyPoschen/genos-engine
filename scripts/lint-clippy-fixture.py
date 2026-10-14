#!/usr/bin/env python3
"""The disallowed-method bans must fire on the fixture and stay quiet on the pass crate."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def clippy(manifest: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "cargo",
            "clippy",
            "--offline",
            "--manifest-path",
            str(manifest),
            "--message-format=short",
            "--",
            "-D",
            "warnings",
        ],
        text=True,
        capture_output=True,
    )


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    fail = clippy(root / "lint" / "fixture" / "fail" / "Cargo.toml")
    text = fail.stdout + fail.stderr
    needed = ("std::thread::sleep", "std::env::var", "std::env::var_os")
    if fail.returncode == 0 or any(name not in text for name in needed):
        sys.stderr.write(text)
        print("fixture: env::var / thread::sleep did not fail clippy", file=sys.stderr)
        return 1
    passed = clippy(root / "lint" / "fixture" / "pass" / "Cargo.toml")
    if passed.returncode != 0:
        sys.stderr.write(passed.stdout)
        sys.stderr.write(passed.stderr)
        print("fixture: the pass crate failed clippy", file=sys.stderr)
        return passed.returncode or 1
    print("clippy fixture: disallowed methods fire, pass crate is clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
