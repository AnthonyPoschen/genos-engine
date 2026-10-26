#!/usr/bin/env python3
"""Run the semgrep rules and require each one to flag its failing example.

The repository scan skips `lint/fail`, which holds the examples on purpose.
`lint/pass` is scanned with the repository and must stay clean.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

RULES = (
    "genos-no-home-path",
    "genos-debug-output-dir",
    "genos-gpu-len-cast",
    "genos-no-gi-v1-term",
    "genos-no-hardcoded-display",
    "genos-no-hyprctl",
    "genos-no-process-exit",
    "genos-no-device-teardown",
)


def semgrep() -> str:
    found = shutil.which("semgrep")
    if found:
        return found
    local = Path.home() / ".local" / "bin" / "semgrep"
    if local.exists():
        return str(local)
    print("semgrep is not on PATH", file=sys.stderr)
    return ""


def run(cmd: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, check=False, text=True, capture_output=True)


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    binary = semgrep()
    if not binary:
        return 1
    config = str(root / "lint" / "semgrep" / "genos.yml")
    repo = run(
        [
            binary,
            "--metrics=off",
            "--error",
            "--quiet",
            "--config",
            config,
            "--exclude",
            "lint/fail",
            "--exclude",
            "target",
            str(root),
        ]
    )
    if repo.returncode != 0:
        sys.stderr.write(repo.stdout)
        sys.stderr.write(repo.stderr)
        print("semgrep: repository scan failed", file=sys.stderr)
        return repo.returncode or 1

    examples = run(
        [
            binary,
            "--metrics=off",
            "--quiet",
            "--json",
            "--config",
            config,
            str(root / "lint" / "fail"),
        ]
    )
    if examples.returncode not in (0, 1):
        sys.stderr.write(examples.stderr)
        print("semgrep: failing-example scan errored", file=sys.stderr)
        return examples.returncode or 1
    try:
        payload = json.loads(examples.stdout or "{}")
    except json.JSONDecodeError as err:
        print(f"semgrep: bad json: {err}", file=sys.stderr)
        sys.stderr.write(examples.stdout)
        sys.stderr.write(examples.stderr)
        return 1
    seen = {hit.get("check_id", "").split(".")[-1] for hit in payload.get("results", [])}
    missing = [rule for rule in RULES if rule not in seen]
    if missing:
        print(f"semgrep: failing examples did not trip {missing}", file=sys.stderr)
        print("saw", sorted(seen), file=sys.stderr)
        return 1
    print(f"semgrep: {len(RULES)} rules, each failing example matched")
    return 0


if __name__ == "__main__":
    sys.exit(main())
