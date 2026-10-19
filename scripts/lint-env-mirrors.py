#!/usr/bin/env python3
"""Fail when Rust names an environment variable the registry does not list.

`lint/env_mirrors.toml` is the list. A `GENOS_*` or `OCC_*` string in Rust
must appear there. `kind = "todo"` is the backlog of switches that still
have no command-line argument. Do not add a new todo. Add a mirror, with
the argument in `detail`, or do not read the variable.

`--self-test` checks the repository, then checks that `lint/fail/env_switch.rs`
names a variable the registry rejects and that `lint/pass` does not.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

NAME = re.compile(r'"((?:GENOS_|OCC_)[A-Z0-9_]+)"')
ENTRY = re.compile(
    r'name = "([^"]+)"\s+kind = "(mirror|todo|override|os|build|test)"',
    re.MULTILINE,
)
KINDS = {"mirror", "todo", "override", "os", "build", "test"}
SKIP_PARTS = {"target", ".git", "lint"}


def registry(path: Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8")
    found = {name: kind for name, kind in ENTRY.findall(text)}
    if not found:
        raise SystemExit(f"{path}: no [[var]] entries")
    unknown = sorted(set(found.values()) - KINDS)
    if unknown:
        raise SystemExit(f"{path}: unknown kinds {unknown}")
    return found


def rust_files(root: Path) -> list[Path]:
    files = []
    for path in root.rglob("*.rs"):
        if any(part in SKIP_PARTS for part in path.parts):
            continue
        files.append(path)
    return files


def names_in(paths: list[Path]) -> dict[str, list[str]]:
    found: dict[str, list[str]] = {}
    for path in paths:
        text = path.read_text(encoding="utf-8", errors="replace")
        for name in NAME.findall(text):
            found.setdefault(name, []).append(str(path))
    return found


def check_tree(root: Path, known: dict[str, str]) -> list[str]:
    errors = []
    used = names_in(rust_files(root))
    for name, paths in sorted(used.items()):
        if name not in known:
            errors.append(f"unregistered {name} in {paths[0]}")
    for name, kind in sorted(known.items()):
        if name not in used and kind in {"mirror", "todo", "override", "test"}:
            errors.append(f"registry {name} ({kind}) is not read by any Rust file")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    known = registry(root / "lint" / "env_mirrors.toml")
    errors = check_tree(root, known)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    if not args.self_test:
        todos = sum(1 for kind in known.values() if kind == "todo")
        print(f"env mirrors: {len(known)} names, {todos} todos")
        return 0

    fail = root / "lint" / "fail" / "env_switch.rs"
    fail_names = names_in([fail])
    if "GENOS_NOT_A_MIRROR" not in fail_names:
        print("self-test: lint/fail/env_switch.rs did not name GENOS_NOT_A_MIRROR", file=sys.stderr)
        return 1
    if "GENOS_NOT_A_MIRROR" in known:
        print("self-test: GENOS_NOT_A_MIRROR is registered and must not be", file=sys.stderr)
        return 1
    pass_names = names_in(list((root / "lint" / "pass").rglob("*.rs")))
    stray = sorted(set(pass_names) - set(known))
    if stray:
        print(f"self-test: lint/pass names an unregistered variable {stray}", file=sys.stderr)
        return 1
    print(f"env mirrors: {len(known)} names, self-test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
