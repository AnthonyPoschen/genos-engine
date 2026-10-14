"""Commit identity and merge-tree conflict checks. No GPU."""

from __future__ import annotations

import subprocess
from pathlib import Path

from lib.contract import Broken


def git(repo: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=repo,
        check=False,
        text=True,
        capture_output=True,
    )


def rev_parse(repo: Path, ref: str) -> str:
    proc = git(repo, ["rev-parse", "--verify", ref])
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip() or ref
        raise Broken(f"unknown ref {detail}")
    sha = proc.stdout.strip()
    if not sha:
        raise Broken(f"unknown ref {ref}")
    return sha


def head_sha(repo: Path, explicit: str | None) -> str:
    return rev_parse(repo, explicit or "HEAD")


def merge_conflicts(repo: Path, base: str, head: str) -> tuple[bool, str]:
    """True when `git merge-tree --write-tree` reports a conflict."""
    proc = git(repo, ["merge-tree", "--write-tree", "--name-only", base, head])
    log = (proc.stdout or "") + (proc.stderr or "")
    if proc.returncode == 0:
        return False, log
    if proc.returncode == 1:
        return True, log
    detail = log.strip() or f"merge-tree exit {proc.returncode}"
    raise Broken(detail)
