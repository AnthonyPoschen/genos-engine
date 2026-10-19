#!/usr/bin/env python3
"""Named constants for shader numbers.

A fractional literal that is not 0 or 1, on a line that does not declare
`const` or `#define`, is a magic number. Existing shaders are listed in
`lint/shader-magic-baseline.txt` with the count they already have. A higher
count, or a new shader with any, fails. `lint/fail` is the example that must
trip; `lint/pass` must stay clean.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

EXTS = {".glsl", ".comp", ".vert", ".frag"}
LITERAL = re.compile(
    r"(?<![A-Za-z0-9_.])(-?(?:\d+\.\d+|\d+\.|\.\d+)(?:[eE][+-]?\d+)?)(?![A-Za-z0-9_.])"
)
NAMED = re.compile(r"^\s*(?:const\b|#define\b)")


def root() -> Path:
    return Path(__file__).resolve().parents[1]


def strip_comments(text: str) -> str:
    out: list[str] = []
    i = 0
    while i < len(text):
        if text.startswith("//", i):
            end = text.find("\n", i)
            if end < 0:
                break
            out.append("\n")
            i = end + 1
            continue
        if text.startswith("/*", i):
            end = text.find("*/", i + 2)
            if end < 0:
                break
            out.append(" " * (end + 2 - i))
            i = end + 2
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def magic_count(text: str) -> int:
    """Fractional literals that are not exactly 0 or 1, outside a const or #define line."""
    total = 0
    for line in strip_comments(text).splitlines():
        if NAMED.match(line):
            continue
        for match in LITERAL.finditer(line):
            try:
                value = float(match.group(1))
            except ValueError:
                total += 1
                continue
            if value in {0.0, 1.0, -1.0}:
                continue
            total += 1
    return total


def scan(directory: Path) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in sorted(directory.rglob("*")):
        if path.suffix not in EXTS or not path.is_file():
            continue
        relative = path.relative_to(directory)
        if "target" in relative.parts or "lint" in relative.parts:
            continue
        count = magic_count(path.read_text(encoding="utf-8", errors="replace"))
        if count:
            counts[str(relative)] = count
    return counts


def load_baseline(path: Path) -> dict[str, int]:
    counts: dict[str, int] = {}
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        name, count = line.rsplit(" ", 1)
        counts[name] = int(count)
    return counts


def main() -> int:
    workspace = root()
    fail = magic_count((workspace / "lint" / "fail" / "shader_magic.comp").read_text())
    if fail == 0:
        print("shader rule: lint/fail/shader_magic.comp did not trip", file=sys.stderr)
        return 1
    passed = magic_count((workspace / "lint" / "pass" / "shader_named.glsl").read_text())
    if passed:
        print(f"shader rule: lint/pass/shader_named.glsl tripped on {passed}", file=sys.stderr)
        return 1
    baseline_path = workspace / "lint" / "shader-magic-baseline.txt"
    baseline = load_baseline(baseline_path)
    found = scan(workspace)
    over: list[str] = []
    under: list[str] = []
    for name in sorted(set(baseline) | set(found)):
        have = found.get(name, 0)
        allowed = baseline.get(name, 0)
        if have > allowed:
            over.append(f"{name}: {have} magic numbers, baseline {allowed}")
        elif have < allowed:
            under.append(f"{name}: {have} magic numbers, baseline {allowed}")
    total = sum(found.values())
    print(f"shader constants: {total} magic numbers (warn), baseline {sum(baseline.values())}")
    if over or under:
        for line in over:
            print(f"new shader magic number: {line}", file=sys.stderr)
        for line in under:
            print(f"baseline is high: {line}", file=sys.stderr)
        return 1
    print("shader constants: failing example tripped, pass shader is clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
