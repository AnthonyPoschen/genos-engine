#!/usr/bin/env python3
"""Proposal: fail a pull request that raises a lint baseline ceiling.

This is a proposal for Anthony to confirm. It is not a confirmed rule and it
is not a standing permission to raise a ceiling. The default label
`baseline-growth-approved` is not a label this repository already has, and
this script does not create one.

An added entry is a path whose last-wins count is higher at the pull request
head than at the merge-base of ``--base-sha`` (the pull request's base SHA)
and ``--head-sha``. A renamed path counts as an addition. A lower count, a
deleted path, a reorder, or a duplicate that does not change the last-wins
count passes.

When anything was added, both of these are required:

* the pull request carries the label from ``--label``
* the previous line of that entry is ``# owner: <who> — <why>``

``--label`` wins over ``GENOS_BASELINE_GROWTH_LABEL``. The environment
variable applies only when ``--label`` is omitted. The owner line is a ``#``
comment, which ``lint-clippy.py`` and ``lint-shader-constants.py`` already
skip, so the ceiling those scripts enforce does not change.

The pulls API is contacted only when an entry was added and ``--labels`` was
omitted. An HTTP error, a timeout, a missing token, or a body that is not a
pull request object fails the run closed. The pull request body is not the
why.

``--self-test`` runs the regression tests and does not read a pull request.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path

PROPOSED_LABEL = "baseline-growth-approved"
ENV_LABEL = "GENOS_BASELINE_GROWTH_LABEL"
EM_DASH = "\u2014"
OWNER_RE = re.compile(
    rf"^#\s*owner:\s*(\S(?:.*?\S)?)\s+{EM_DASH}\s*(\S(?:.*\S)?)\s*$"
)
SHA_RE = re.compile(r"^[0-9a-fA-F]{40}(?:[0-9a-fA-F]{24})?$")
REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
BASELINE_RE = re.compile(r"^lint/(?:.+/)?[^/]*baseline[^/]*\.txt$")

ROOT = Path(__file__).resolve().parents[1]


class ParseError(Exception):
    """A baseline file does not match the linters' parser."""


class MetadataError(Exception):
    """Pull request metadata could not be read. The run fails closed."""


class GitError(Exception):
    """The base and head could not be compared. The run fails closed."""


@dataclass(frozen=True)
class Added:
    baseline: str
    path: str
    old: int
    new: int
    previous_line: str | None


@dataclass(frozen=True)
class Outcome:
    code: int
    messages: tuple[str, ...]


def owner_parts(line: str | None) -> tuple[str, str] | None:
    """Return who and why when `line` is the proposed owner comment."""
    if line is None:
        return None
    match = OWNER_RE.match(line.strip())
    if match is None:
        return None
    who, why = match.group(1).strip(), match.group(2).strip()
    if not who or not why:
        return None
    return who, why


_LOADERS: tuple | None = None


def _module_loader(filename: str):
    path = ROOT / "scripts" / filename
    spec = importlib.util.spec_from_file_location(f"genos_{filename}", path)
    if spec is None or spec.loader is None:
        raise ParseError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.load_baseline


def _loaders():
    global _LOADERS
    if _LOADERS is None:
        _LOADERS = (
            _module_loader("lint-clippy.py"),
            _module_loader("lint-shader-constants.py"),
        )
    return _LOADERS


def _run_loader(loader, text: str) -> dict[str, int]:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "baseline.txt"
        path.write_text(text, encoding="utf-8")
        try:
            counts = loader(path)
        except (OSError, ValueError) as exc:
            raise ParseError(str(exc)) from exc
    if not isinstance(counts, dict):
        raise ParseError("baseline parser did not return a map")
    cleaned: dict[str, int] = {}
    for name, count in counts.items():
        if not isinstance(name, str) or isinstance(count, bool) or not isinstance(count, int):
            raise ParseError("baseline parser returned an unexpected row")
        cleaned[name] = count
    return cleaned


def effective_counts(text: str) -> dict[str, int]:
    """Last-wins ceilings, requiring both linters to agree.

    The two baseline parsers skip blank lines and ``#`` comments. A trailing
    note on an entry line is a parse error here, because it is a parse error
    there. Agreement is the check that an owner comment did not change the
    ceiling.
    """
    clippy_load, shader_load = _loaders()
    clippy = _run_loader(clippy_load, text)
    shader = _run_loader(shader_load, text)
    if clippy != shader:
        raise ParseError(
            "lint-clippy.py and lint-shader-constants.py parsed different ceilings"
        )
    return clippy


def previous_lines(text: str) -> dict[str, str | None]:
    """Previous line of each last-wins entry. Blank lines break 'beside'."""
    owners: dict[str, str | None] = {}
    previous: str | None = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            previous = None
            continue
        if line.startswith("#"):
            previous = line
            continue
        try:
            name, count = line.rsplit(" ", 1)
            int(count)
        except ValueError as exc:
            raise ParseError(str(exc)) from exc
        owners[name] = previous
        previous = None
    return owners


def additions_in_text(base: str, head: str, baseline: str) -> list[Added]:
    base_counts = effective_counts(base)
    head_counts = effective_counts(head)
    previous = previous_lines(head)
    if set(previous) != set(head_counts):
        raise ParseError(f"{baseline}: owner scan does not match the linters")
    added: list[Added] = []
    for path in sorted(head_counts):
        old = base_counts.get(path, 0)
        new = head_counts[path]
        if new > old:
            added.append(Added(baseline, path, old, new, previous.get(path)))
    return added


def judge(added: list[Added], labels: set[str], required_label: str) -> Outcome:
    if not added:
        return Outcome(0, ("baseline-growth: no added entries.",))
    problems: list[str] = []
    for item in added:
        if owner_parts(item.previous_line) is None:
            problems.append(
                f"{item.baseline}: {item.path} rose from {item.old} to {item.new} "
                f"without `# owner: <who> {EM_DASH} <why>` on the previous line"
            )
    if required_label not in labels:
        names = ", ".join(f"{item.path}" for item in added)
        problems.append(
            f"missing label `{required_label}` for {names}. "
            f"The default name {PROPOSED_LABEL} is a proposal; this check does not create it."
        )
    if problems:
        header = (
            "baseline-growth: proposal (not a confirmed rule, not a standing permission). "
            "An added entry needs the label and an owner line."
        )
        return Outcome(1, (header, *problems))
    return Outcome(
        0,
        (
            f"baseline-growth: {len(added)} added entries, label `{required_label}` present, "
            "owner lines present. proposal contract satisfied.",
        ),
    )


def _git(repo: Path, args: list[str]) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *args],
        cwd=repo,
        check=False,
        capture_output=True,
    )


def _require_sha(value: str, role: str) -> str:
    if not SHA_RE.fullmatch(value or ""):
        raise GitError(f"{role} is not a commit sha. Fail closed.")
    return value


def _merge_base(repo: Path, base_sha: str, head_sha: str) -> str:
    proc = _git(repo, ["merge-base", base_sha, head_sha])
    if proc.returncode != 0:
        detail = proc.stderr.decode("utf-8", errors="replace").strip().splitlines()
        tail = detail[-1] if detail else "merge-base failed"
        raise GitError(f"no merge base ({tail}). Fail closed.")
    sha = proc.stdout.decode("utf-8", errors="replace").strip()
    if not SHA_RE.fullmatch(sha):
        raise GitError("merge-base did not return a sha. Fail closed.")
    return sha


def _is_baseline(path: str) -> bool:
    if not path or path.startswith("/") or "\\" in path:
        return False
    if any(part in {"", ".", ".."} for part in path.split("/")):
        return False
    return BASELINE_RE.fullmatch(path) is not None


def _list_baselines(repo: Path, sha: str) -> set[str]:
    proc = _git(repo, ["ls-tree", "-r", "--name-only", sha, "lint"])
    if proc.returncode != 0:
        raise GitError("cannot list lint files. Fail closed.")
    found: set[str] = set()
    for raw in proc.stdout.decode("utf-8", errors="replace").splitlines():
        if _is_baseline(raw):
            found.add(raw)
    return found


def _rename_map(repo: Path, base_sha: str, head_sha: str) -> dict[str, str]:
    # 20% keeps a short baseline that gained a line paired with its old path.
    # A rewrite below that is a new file, so every entry is an addition.
    proc = _git(
        repo,
        ["diff", "-M20%", "--name-status", base_sha, head_sha],
    )
    if proc.returncode != 0:
        raise GitError("cannot read renames. Fail closed.")
    mapping: dict[str, str] = {}
    for raw in proc.stdout.decode("utf-8", errors="replace").splitlines():
        parts = raw.split("\t")
        if len(parts) != 3 or not parts[0].startswith("R"):
            continue
        old, new = parts[1], parts[2]
        if _is_baseline(old) and _is_baseline(new):
            mapping[old] = new
    return mapping


def _file_at(repo: Path, sha: str, path: str) -> str | None:
    if not _is_baseline(path):
        raise GitError(f"refusing to read {path}. Fail closed.")
    probe = _git(repo, ["cat-file", "-e", f"{sha}:{path}"])
    if probe.returncode != 0:
        return None
    proc = _git(repo, ["show", f"{sha}:{path}"])
    if proc.returncode != 0:
        raise GitError(f"cannot read {path}. Fail closed.")
    if b"\0" in proc.stdout:
        raise ParseError(f"{path} is not text. Fail closed.")
    return proc.stdout.decode("utf-8")


def _pairs(repo: Path, base_sha: str, head_sha: str) -> list[tuple[str | None, str | None]]:
    base_files = _list_baselines(repo, base_sha)
    head_files = _list_baselines(repo, head_sha)
    renamed = _rename_map(repo, base_sha, head_sha)
    used_head: set[str] = set()
    pairs: list[tuple[str | None, str | None]] = []
    for old in sorted(base_files):
        new = renamed.get(old)
        if new is None and old in head_files:
            new = old
        if new is not None:
            used_head.add(new)
        pairs.append((old, new))
    for new in sorted(head_files - used_head):
        pairs.append((None, new))
    return pairs


def collect(repo: Path, base_sha: str, head_sha: str) -> tuple[str, list[Added]]:
    base_sha = _require_sha(base_sha, "base sha")
    head_sha = _require_sha(head_sha, "head sha")
    merge = _merge_base(repo, base_sha, head_sha)
    added: list[Added] = []
    for old, new in _pairs(repo, merge, head_sha):
        old_text = "" if old is None else (_file_at(repo, merge, old) or "")
        new_text = "" if new is None else (_file_at(repo, head_sha, new) or "")
        label = new or old or "baseline"
        added.extend(additions_in_text(old_text, new_text, label))
    return merge, added


def evaluate(
    repo: Path,
    base_sha: str,
    head_sha: str,
    labels: set[str] | None,
    required_label: str,
    fetch_labels: Callable[[], set[str]] | None,
) -> Outcome:
    try:
        merge, added = collect(repo, base_sha, head_sha)
    except GitError as exc:
        return Outcome(2, (f"baseline-growth: {exc}",))
    except ParseError as exc:
        return Outcome(1, (f"baseline-growth: {exc}. Fail closed.",))
    if not added:
        return Outcome(0, (f"baseline-growth: no added entries relative to {merge}.",))
    if labels is None:
        if fetch_labels is None:
            return Outcome(
                2,
                ("baseline-growth: pull request metadata failed (no label source). Fail closed.",),
            )
        try:
            labels = fetch_labels()
        except MetadataError as exc:
            return Outcome(2, (f"baseline-growth: {exc}",))
    return judge(added, labels, required_label)


def resolve_label(flag: str | None, env: Mapping[str, str]) -> str:
    if flag is not None:
        name = flag.strip()
        if not name:
            raise MetadataError("--label is empty. Fail closed.")
        return name
    mirrored = env.get(ENV_LABEL, "").strip()
    if mirrored:
        return mirrored
    return PROPOSED_LABEL


def _api_base_ok(api_base: str) -> bool:
    from urllib.parse import urlparse

    parsed = urlparse(api_base)
    host = (parsed.hostname or "").lower()
    if parsed.scheme == "https" and host:
        return True
    return parsed.scheme == "http" and host in {"127.0.0.1", "localhost"}


def fetch_pull_labels(repo: str, pull: int, token: str, api_base: str) -> set[str]:
    if not REPO_RE.fullmatch(repo or ""):
        raise MetadataError("repository is not owner/name. Fail closed.")
    if pull < 1:
        raise MetadataError("pull number is missing. Fail closed.")
    if not token:
        raise MetadataError("pull request metadata failed (no token). Fail closed.")
    if not _api_base_ok(api_base):
        raise MetadataError("api base is not https. Fail closed.")
    url = f"{api_base.rstrip('/')}/repos/{repo}/pulls/{pull}"
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "User-Agent": "genos-baseline-growth",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            status = getattr(response, "status", 200)
            raw = response.read()
    except urllib.error.HTTPError as exc:
        raise MetadataError(
            f"pull request metadata failed (HTTP {exc.code}). Fail closed."
        ) from exc
    except Exception as exc:
        raise MetadataError(
            f"pull request metadata failed ({exc.__class__.__name__}). Fail closed."
        ) from exc
    if status != 200:
        raise MetadataError(f"pull request metadata failed (HTTP {status}). Fail closed.")
    try:
        body = json.loads(raw.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise MetadataError(
            f"pull request metadata failed ({exc.__class__.__name__}). Fail closed."
        ) from exc
    if not isinstance(body, dict) or "labels" not in body or not isinstance(body["labels"], list):
        raise MetadataError("pull request metadata failed (no labels). Fail closed.")
    names: set[str] = set()
    for item in body["labels"]:
        if not isinstance(item, dict) or not isinstance(item.get("name"), str) or not item["name"]:
            raise MetadataError("pull request metadata failed (bad label). Fail closed.")
        names.add(item["name"])
    return names


def _token(env: Mapping[str, str]) -> str:
    return env.get("GITHUB_TOKEN", "").strip() or env.get("GH_TOKEN", "").strip()


def _print_outcome(outcome: Outcome) -> int:
    stream = sys.stdout if outcome.code == 0 else sys.stderr
    print("\n".join(outcome.messages), file=stream)
    return outcome.code


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true", help="run the regression tests and exit")
    parser.add_argument("--cwd", default=".", help="repository to compare (default: the current directory)")
    parser.add_argument("--base-sha", help="pull request base SHA from the workflow event")
    parser.add_argument("--head-sha", help="pull request head SHA from the workflow event")
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""), help="owner/name for the pulls API")
    parser.add_argument("--pull", type=int, help="pull request number for the pulls API")
    parser.add_argument(
        "--label",
        default=None,
        help=(
            "label that must be present when an entry is added. "
            f"Wins over ${ENV_LABEL}. Default: ${ENV_LABEL} if set, otherwise the proposed "
            f"name {PROPOSED_LABEL}."
        ),
    )
    parser.add_argument(
        "--labels",
        default=None,
        help="comma-separated labels already on the pull request. When set, the API is not called.",
    )
    parser.add_argument(
        "--api-base",
        default=os.environ.get("GITHUB_API_URL", "https://api.github.com"),
        help="pulls API origin. https, or http on localhost for tests. Default: $GITHUB_API_URL or https://api.github.com",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.self_test:
        proc = subprocess.run(
            [sys.executable, str(ROOT / "scripts" / "tests" / "test_baseline_growth.py")],
            check=False,
        )
        return proc.returncode
    try:
        required = resolve_label(args.label, os.environ)
    except MetadataError as exc:
        print(f"baseline-growth: {exc}", file=sys.stderr)
        return 2
    if not args.base_sha or not args.head_sha:
        print("baseline-growth: missing base or head sha. Fail closed.", file=sys.stderr)
        return 2
    labels: set[str] | None
    fetch: Callable[[], set[str]] | None
    if args.labels is None:
        labels = None

        def fetch() -> set[str]:
            if args.pull is None:
                raise MetadataError("pull request metadata failed (no pull number). Fail closed.")
            return fetch_pull_labels(args.repo, args.pull, _token(os.environ), args.api_base)
    else:
        labels = {part.strip() for part in args.labels.split(",") if part.strip()}
        fetch = None
    outcome = evaluate(
        Path(args.cwd).resolve(),
        args.base_sha,
        args.head_sha,
        labels,
        required,
        fetch,
    )
    return _print_outcome(outcome)


if __name__ == "__main__":
    sys.exit(main())
