#!/usr/bin/env python3
"""Check docs/systems/*/plan.md for a References section and local links.

A plan fails when it has no `## References` heading, or when a markdown link
points at a file that is not in the repository. `docs/systems/core-networking/plan.md`
is allowlisted until that plan lands. http(s) links are left alone.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import tempfile
from pathlib import Path
from urllib.parse import unquote

# Repo-relative paths that plans may link before the file exists.
ALLOW_MISSING = {
    "docs/systems/core-networking/plan.md",
}

HEADING = re.compile(r"^## References\s*$", re.MULTILINE)
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def link_path(raw: str) -> str | None:
    """Return a local path, or None when the destination is not a file link."""
    text = raw.strip()
    if not text:
        return None
    if text.startswith("<"):
        end = text.find(">")
        text = text[1:end] if end != -1 else text[1:]
    else:
        text = text.split()[0]
    text = unquote(text).strip()
    text = text.split("#", 1)[0].split("?", 1)[0].strip()
    if not text:
        return None
    if text.startswith(("http://", "https://", "mailto:", "//")):
        return None
    return text


def repo_relative(root: Path, markdown: Path, link: str) -> str | None:
    """Normalize `link` against `markdown`. None means the path leaves `root`."""
    if link.startswith("/"):
        candidate = root / link.lstrip("/")
    else:
        candidate = markdown.parent / link
    relative = os.path.relpath(os.path.normpath(candidate), root)
    if relative.startswith("..") or os.path.isabs(relative):
        return None
    return relative.replace("\\", "/")


def check(root: Path) -> list[str]:
    errors: list[str] = []
    plans = sorted((root / "docs" / "systems").glob("*/plan.md"))
    if not plans:
        return ["no docs/systems/*/plan.md files"]
    for plan in plans:
        text = plan.read_text(encoding="utf-8")
        shown = plan.relative_to(root).as_posix()
        if HEADING.search(text) is None:
            errors.append(f"{shown}: missing a ## References section")
        for match in LINK.finditer(text):
            target = link_path(match.group(1))
            if target is None:
                continue
            relative = repo_relative(root, plan, target)
            if relative is None:
                errors.append(f"{shown}: link leaves the repository: {target}")
                continue
            if (root / relative).exists():
                continue
            if relative in ALLOW_MISSING:
                continue
            errors.append(f"{shown}: missing file {relative}")
    return errors


def self_test() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        systems = root / "docs" / "systems"
        good = systems / "widgets"
        good.mkdir(parents=True)
        (good / "note.md").write_text("ok\n", encoding="utf-8")
        (systems / "rendering").mkdir()
        # A different filename is outside the glob.
        (systems / "rendering" / "extras-plan.md").write_text(
            "no references here\n", encoding="utf-8"
        )

        bare = systems / "bare"
        bare.mkdir()
        (bare / "plan.md").write_text("# Bare\n\nNo sources.\n", encoding="utf-8")
        errors = check(root)
        if not any("bare/plan.md: missing a ## References" in item for item in errors):
            raise SystemExit(f"self-test: missing References did not fail: {errors}")

        broken = systems / "broken"
        broken.mkdir()
        (broken / "plan.md").write_text(
            "# Broken\n\n## References\n\n- [gone](missing.md)\n",
            encoding="utf-8",
        )
        errors = check(root)
        if not any("broken/plan.md: missing file" in item for item in errors):
            raise SystemExit(f"self-test: missing file did not fail: {errors}")

        (broken / "plan.md").write_text(
            "# Broken\n\n## References\n\n- [net](../core-networking/plan.md)\n"
            "- [note](../widgets/note.md)\n"
            "- [web](https://example.com/missing)\n",
            encoding="utf-8",
        )
        (bare / "plan.md").write_text(
            "# Bare\n\n## References\n\n- [note](../widgets/note.md)\n",
            encoding="utf-8",
        )
        errors = check(root)
        if errors:
            raise SystemExit(f"self-test: allowlisted plan failed: {errors}")

        (good / "plan.md").write_text(
            "# Widgets\n\n## References\n\n- [out](../../../../../../etc/passwd)\n",
            encoding="utf-8",
        )
        errors = check(root)
        if not any("leaves the repository" in item for item in errors):
            raise SystemExit(f"self-test: escaping link did not fail: {errors}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check the failure and allowlist cases in a temp tree",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root (default: parent of this script)",
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("lint-plan-docs self-test passed")
        return 0
    errors = check(args.root)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"lint-plan-docs: {len(list((args.root / 'docs' / 'systems').glob('*/plan.md')))} plan(s) ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
