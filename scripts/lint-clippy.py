#!/usr/bin/env python3
"""Workspace clippy, plus a ceiling on lossy `as` casts.

`cast_possible_truncation`, `cast_possible_wrap`, and `cast_sign_loss` are
warnings. `--force-warn` still reports the lines that files already
`expect`, so this script can see them. A file may not have more unique lines
of those lints than `lint/lossy-cast-baseline.txt`. Every other warning or
error fails the run.
"""

from __future__ import annotations

import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

CASTS = {
    "clippy::cast_possible_truncation",
    "clippy::cast_sign_loss",
    "clippy::cast_possible_wrap",
}


def root() -> Path:
    return Path(__file__).resolve().parents[1]


def load_baseline(path: Path) -> dict[str, int]:
    counts: dict[str, int] = {}
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        file_name, count = line.rsplit(" ", 1)
        counts[file_name] = int(count)
    return counts


def sites_from_messages(stream: str) -> tuple[dict[str, int], list[str]]:
    """Return unique lossy-cast lines per file, and any other diagnostic."""
    per_file: dict[str, set[int]] = defaultdict(set)
    problems: list[str] = []
    for raw in stream.splitlines():
        if not raw.startswith("{"):
            continue
        try:
            event = json.loads(raw)
        except json.JSONDecodeError:
            problems.append(f"bad json: {raw[:160]}")
            continue
        if event.get("reason") != "compiler-message":
            continue
        message = event.get("message") or {}
        level = message.get("level")
        if level not in {"warning", "error"}:
            continue
        code = (message.get("code") or {}).get("code")
        if code in CASTS and level == "warning":
            spans = message.get("spans") or []
            primary = next((span for span in spans if span.get("is_primary")), None)
            if primary is None:
                continue
            file_name = primary.get("file_name") or ""
            workspace = str(root()) + "/"
            if file_name.startswith(workspace):
                file_name = file_name[len(workspace) :]
            per_file[file_name].add(int(primary["line_start"]))
            continue
        rendered = (message.get("rendered") or message.get("message") or "").strip()
        problems.append(rendered.splitlines()[0] if rendered else f"{level}: {code}")
    return {name: len(lines) for name, lines in per_file.items()}, problems


def main() -> int:
    workspace = root()
    baseline_path = workspace / "lint" / "lossy-cast-baseline.txt"
    if not baseline_path.is_file():
        print(f"missing {baseline_path}", file=sys.stderr)
        return 1
    baseline = load_baseline(baseline_path)
    command = [
        "cargo",
        "clippy",
        "--workspace",
        "--all-targets",
        "--message-format=json",
        "--offline",
        "--",
        "-D",
        "warnings",
        "--force-warn",
        "clippy::cast_possible_truncation",
        "--force-warn",
        "clippy::cast_possible_wrap",
        "--force-warn",
        "clippy::cast_sign_loss",
    ]
    run = subprocess.run(command, cwd=workspace, text=True, capture_output=True)
    sys.stderr.write(run.stderr)
    found, problems = sites_from_messages(run.stdout)
    if run.returncode != 0 and not problems:
        problems.append(f"cargo clippy exited {run.returncode}")
    over: list[str] = []
    under: list[str] = []
    for name in sorted(set(baseline) | set(found)):
        have = found.get(name, 0)
        allowed = baseline.get(name, 0)
        if have > allowed:
            over.append(f"{name}: {have} lossy casts, baseline {allowed}")
        elif have < allowed:
            under.append(f"{name}: {have} lossy casts, baseline {allowed}")
    total = sum(found.values())
    print(f"lossy casts: {total} unique lines (warn), baseline {sum(baseline.values())}")
    if problems or over or under:
        for line in problems:
            print(line, file=sys.stderr)
        for line in over:
            print(f"new lossy cast: {line}", file=sys.stderr)
        for line in under:
            print(f"baseline is high: {line}", file=sys.stderr)
        print("docs/LINT_RULES.md lists the burndown. Lower the baseline only when the casts are gone.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
