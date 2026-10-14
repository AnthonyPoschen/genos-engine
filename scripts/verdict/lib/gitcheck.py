"""Commit identity and merge-tree conflict checks. No GPU.

Branch names resolve to `origin/<name>` after `git fetch origin`. A local
branch that is behind its upstream is refused, so a stale `gi-v2` is not built.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

from lib.contract import Broken

_FETCHED: set[str] = set()
_SHA = re.compile(r"^[0-9a-fA-F]{7,40}$")


def git(repo: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=repo,
        check=False,
        text=True,
        capture_output=True,
    )


def fetch_origin(repo: Path) -> None:
    """Update `origin/*` once per repository. A repo with no origin is left alone."""
    key = str(repo.resolve())
    if key in _FETCHED:
        return
    listed = git(repo, ["remote"])
    if listed.returncode != 0:
        detail = (listed.stderr or listed.stdout).strip() or "git remote failed"
        raise Broken(detail)
    if "origin" not in listed.stdout.split():
        _FETCHED.add(key)
        return
    proc = git(repo, ["fetch", "origin"])
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip() or "git fetch origin failed"
        raise Broken(detail)
    _FETCHED.add(key)


def rev_parse(repo: Path, ref: str) -> str:
    proc = git(repo, ["rev-parse", "--verify", ref])
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip() or ref
        raise Broken(f"unknown ref {detail}")
    sha = proc.stdout.strip()
    if not sha:
        raise Broken(f"unknown ref {ref}")
    return sha


def _exists(repo: Path, ref: str) -> bool:
    return git(repo, ["rev-parse", "--verify", "--quiet", ref]).returncode == 0


def _is_sha(repo: Path, name: str) -> bool:
    if _SHA.fullmatch(name) is None:
        return False
    proc = git(repo, ["rev-parse", "--verify", f"{name}^{{commit}}"])
    if proc.returncode != 0:
        return False
    return proc.stdout.strip().lower().startswith(name.lower())


def _upstream(repo: Path, branch: str) -> str | None:
    proc = git(repo, ["rev-parse", "--abbrev-ref", f"{branch}@{{upstream}}"])
    if proc.returncode != 0:
        return None
    name = proc.stdout.strip()
    if not name or name == "@{upstream}":
        return None
    return name


def _is_ancestor(repo: Path, older: str, newer: str) -> bool:
    return git(repo, ["merge-base", "--is-ancestor", older, newer]).returncode == 0


def _local_branch(repo: Path, branch: str) -> str:
    """Use a local branch only when it is not behind its upstream."""
    local_ref = f"refs/heads/{branch}"
    if not _exists(repo, local_ref):
        return rev_parse(repo, branch)
    local = rev_parse(repo, local_ref)
    upstream = _upstream(repo, branch)
    if upstream is None:
        return local
    remote = rev_parse(repo, upstream)
    if local == remote:
        return local
    if _is_ancestor(repo, local, remote) and not _is_ancestor(repo, remote, local):
        raise Broken(f"{branch} is behind {upstream}")
    if not _is_ancestor(repo, remote, local):
        raise Broken(f"{branch} is behind {upstream}")
    return local


def resolve_ref(repo: Path, name: str) -> str:
    """Fetch, then resolve a branch name to `origin/<name>`.

    `HEAD` stays the checkout. A commit sha stays that commit. `origin/gi-v2`
    and `refs/remotes/...` stay those refs. `refs/heads/<name>` is the local
    branch and is refused when that branch is behind its upstream.
    """
    fetch_origin(repo)
    text = name.strip()
    if not text or text == "HEAD" or text.startswith("HEAD~") or text.startswith("HEAD^"):
        return rev_parse(repo, text or "HEAD")
    if _is_sha(repo, text):
        return rev_parse(repo, text)
    if text.startswith("refs/heads/"):
        return _local_branch(repo, text.removeprefix("refs/heads/"))
    if text.startswith("refs/remotes/") or text.startswith("origin/"):
        return rev_parse(repo, text)
    remote = f"origin/{text}"
    if _exists(repo, remote):
        return rev_parse(repo, remote)
    return _local_branch(repo, text)


def head_sha(repo: Path, explicit: str | None) -> str:
    return resolve_ref(repo, explicit or "HEAD")


def sha_lines(pairs: tuple[tuple[str, str], ...]) -> str:
    return "".join(f"{label} {sha}\n" for label, sha in pairs)


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
