#!/usr/bin/env python3
"""Retired glossary terms stay retired (plan C3).

Reads the `### Retired terms` list in docs/lighting/GLOSSARY.md and fails when
the diff under test ADDS a line naming one. Only added lines count, so the
existing sites don't block anyone; the RENAMES.md commits burn those down.

Allowed: the glossary, RENAMES.md and docs/plans (they define and track the
names), alias shims (a line saying `deprecated` or `alias`), and history
notes (a line saying `retired`, `removed` or `gone`).

Diff under test: in GitHub Actions, the PR's base branch to HEAD (pull_request)
or HEAD's first parent (push); locally, merge-base with origin/master, or the
ref in CHECK_TERMS_BASE.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

GLOSSARY = "docs/lighting/GLOSSARY.md"
EXEMPT = (GLOSSARY, "docs/lighting/RENAMES.md", "docs/plans/")
SHIM = re.compile(r"deprecated|alias|retired|removed|\bgone\b", re.I)


def git(*args: str, cwd: Path | None = None, check: bool = True) -> str:
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if check and r.returncode != 0:
        raise SystemExit(f"check-terms: git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


def retired_terms(text: str) -> list[str]:
    m = re.search(r"^### Retired terms.*?$(.*?)(?=^#|^---|\Z)", text, re.M | re.S)
    if not m:
        raise SystemExit(f"check-terms: no '### Retired terms' section in {GLOSSARY}")
    return re.findall(r"^- `([^`]+)`", m.group(1), re.M)


def pattern(term: str) -> re.Pattern[str]:
    return re.compile(r"(?<![A-Za-z0-9_])" + re.escape(term) + r"(?![A-Za-z0-9_])", re.I)


def added_lines(diff: str):
    path = None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("+") and path:
            yield path, line[1:]


def violations(diff: str, terms: list[str]) -> list[str]:
    pats = [(t, pattern(t)) for t in terms]
    out = []
    for path, line in added_lines(diff):
        if path.startswith(EXEMPT) or SHIM.search(line):
            continue
        for term, pat in pats:
            if pat.search(line):
                out.append(f"{path}: adds retired term `{term}`: {line.strip()}")
    return out


def diff_range(root: Path) -> list[str]:
    base = os.environ.get("CHECK_TERMS_BASE")
    if base:
        return [git("merge-base", base, "HEAD", cwd=root).strip(), "HEAD"]
    if os.environ.get("GITHUB_ACTIONS"):
        ref = os.environ.get("GITHUB_BASE_REF")
        if ref:  # HEAD is the PR merge commit; base tip..HEAD is the PR's change
            git("fetch", "--no-tags", "--depth=1", "origin", ref, cwd=root)
            return ["FETCH_HEAD", "HEAD"]
        git("fetch", "--no-tags", "--depth=2", "origin", os.environ["GITHUB_SHA"], cwd=root)
        return ["HEAD^", "HEAD"]
    return [git("merge-base", "origin/master", "HEAD", cwd=root).strip(), "HEAD"]


def self_test() -> None:
    terms = retired_terms("### Retired terms\n- `residency_mb`\n- `GENOS_LIGHTING_SC`\n\n---\n")
    assert terms == ["residency_mb", "GENOS_LIGHTING_SC"], terms
    diff = "\n".join([
        "+++ b/crates/a.rs", "+let x = residency_mb;", "+// residency_mb deprecated alias",
        "+GENOS_LIGHTING_SC_CAP", "+my_residency_mb2", " residency_mb context",
        "+++ b/docs/lighting/RENAMES.md", "+residency_mb",
    ])
    v = violations(diff, terms)
    assert len(v) == 1 and "let x" in v[0], v
    # end to end in a scratch repo
    with tempfile.TemporaryDirectory() as d:
        r = Path(d)
        git("init", "-q", "-b", "master", cwd=r)
        (r / "docs/lighting").mkdir(parents=True)
        (r / GLOSSARY).write_text("### Retired terms\n- `old_name`\n")
        (r / "a.txt").write_text("old_name here before\n")
        for a in (("add", "."), ("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "a")):
            git(*a, cwd=r)
        (r / "a.txt").write_text("old_name here before\nnew_name\n")
        env = dict(os.environ, CHECK_TERMS_BASE="master")
        assert run(r, env) == 0
        (r / "a.txt").write_text("old_name here before\nold_name again\n")
        assert run(r, env) == 1
    print("check-terms: self-test ok")


def run(root: Path, env: dict[str, str] | None = None) -> int:
    saved = os.environ.copy()
    if env is not None:
        os.environ.clear()
        os.environ.update(env)
    try:
        terms = retired_terms((root / GLOSSARY).read_text())
        a, b = diff_range(root)
        # b == HEAD: include uncommitted work locally (worktree diff)
        diff = git("diff", "-U0", "--no-color", a, cwd=root) if not os.environ.get("GITHUB_ACTIONS") \
            else git("diff", "-U0", "--no-color", a, b, cwd=root)
    finally:
        os.environ.clear()
        os.environ.update(saved)
    bad = violations(diff, terms)
    for v in bad:
        print(v, file=sys.stderr)
    if bad:
        print(f"check-terms: {len(bad)} retired term(s) added; use the current name ({GLOSSARY})", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        self_test()
        sys.exit(0)
    sys.exit(run(Path(__file__).resolve().parent.parent))
