"""Verdict cache keyed on the commit sha and an input hash."""

from __future__ import annotations

import json
from pathlib import Path

from lib.contract import CODE, PASS, Verdict, evidence_dir, safe_name


def cache_root(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit)
    return Path("genos-debug") / "verdict" / "cache"


def cache_path(root: Path, sha: str, digest: str) -> Path:
    return root / safe_name(sha) / digest


def load(root: Path, sha: str, digest: str) -> dict | None:
    path = cache_path(root, sha, digest)
    if not path.is_file():
        return None
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("sha") != sha or data.get("input_hash") != digest:
        return None
    status = data.get("status")
    if status not in CODE:
        return None
    return data


def store(root: Path, sha: str, digest: str, status: str, reason: str, evidence: Path) -> None:
    path = cache_path(root, sha, digest)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "sha": sha,
        "input_hash": digest,
        "status": status,
        "reason": reason,
        "code": CODE[status],
        "evidence": str(evidence),
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def replay(record: dict, evidence: Path) -> Verdict:
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence / "cache.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    status = record["status"]
    reason = f"cached {record['reason']}"
    return Verdict(status, reason, evidence, CODE[status])


def default_evidence(kind: str, sha: str, out: str | None) -> Path:
    return evidence_dir(kind, sha, out)


def cached_is_pass(record: dict) -> bool:
    return record.get("status") == PASS
