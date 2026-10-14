#!/usr/bin/env python3
"""New head shas per PR since the last verdict.

PASS when nothing new is waiting, so a watcher stays asleep.
UNSURE lists the new heads in the evidence and the watcher wakes.
Status: probation.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.gitcheck import fetch_origin
from lib.contract import (
    PASS,
    UNSURE,
    Broken,
    Verdict,
    add_common,
    apply_common_mirrors,
    dry_verdict,
    evidence_dir,
    mirror,
    repo_root,
    run_main,
    silence_argv,
    write_args,
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="PR head watcher.")
    parser.add_argument("--state", help="json map of pr number to last head sha")
    parser.add_argument("--prs-file", help="fixture json list of {number, head, url}")
    parser.add_argument("--repo", help="repository for gh (default: this checkout)")
    add_common(parser)
    return parser


def parse(argv: list[str]) -> argparse.Namespace:
    ns = build_parser().parse_args(argv)
    apply_common_mirrors(ns)
    ns.state = mirror(ns.state, "GENOS_VERDICT_PR_STATE")
    return ns


def validate(ns: argparse.Namespace) -> None:
    if ns.dry_run:
        return
    if not ns.prs_file and not ns.state and not ns.out:
        # A live watch still has defaults. Nothing required besides a place to record state.
        return


def load_prs(path: Path) -> list[dict]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise Broken("prs file must be a list")
    rows = []
    for item in data:
        number = str(item["number"])
        head = str(item.get("head") or item.get("headRefOid") or "")
        if not head:
            raise Broken(f"pr {number} has no head")
        rows.append({"number": number, "head": head, "url": str(item.get("url", ""))})
    return rows


def live_prs(repo: Path) -> list[dict]:
    proc = subprocess.run(
        [
            "gh",
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            "number,headRefOid,url",
            "--limit",
            "100",
        ],
        cwd=repo,
        check=False,
        text=True,
        capture_output=True,
    )
    if proc.returncode != 0:
        raise Broken((proc.stderr or proc.stdout).strip() or "gh pr list failed")
    data = json.loads(proc.stdout or "[]")
    return [
        {"number": str(item["number"]), "head": item["headRefOid"], "url": item.get("url", "")}
        for item in data
    ]


def diff_heads(previous: dict[str, str], current: list[dict]) -> list[dict]:
    fresh = []
    for item in current:
        old = previous.get(item["number"])
        if old != item["head"]:
            fresh.append(item)
    return fresh


def decide(ns: argparse.Namespace, argv: list[str], holder: list[Path]) -> Verdict:
    evidence = evidence_dir("pr-watch", ns.sha or "latest", ns.out)
    evidence.mkdir(parents=True, exist_ok=True)
    holder[0] = evidence
    if ns.dry_run:
        return dry_verdict(evidence, argv)
    repo = Path(ns.repo) if ns.repo else repo_root()
    fetch_origin(repo)
    state_path = Path(ns.state) if ns.state else Path("genos-debug/verdict/pr-watch/state.json")
    previous: dict[str, str] = {}
    if state_path.is_file():
        previous = {str(key): str(value) for key, value in json.loads(state_path.read_text(encoding="utf-8")).items()}
    current = load_prs(Path(ns.prs_file)) if ns.prs_file else live_prs(repo)
    fresh = diff_heads(previous, current) if previous else []
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(
        json.dumps({item["number"]: item["head"] for item in current}, indent=2) + "\n",
        encoding="utf-8",
    )
    write_args(evidence, argv)
    lines = [f"{item['number']} {item['head']} {item['url']}".rstrip() for item in fresh]
    (evidence / "new-heads.txt").write_text(("\n".join(lines) + "\n") if lines else "", encoding="utf-8")
    (evidence / "prs.json").write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8")
    if not previous:
        return Verdict(PASS, f"baseline {len(current)} prs", evidence)
    if not fresh:
        return Verdict(PASS, "no new heads", evidence)
    return Verdict(UNSURE, f"new heads count={len(fresh)}", evidence)


def main(argv: list[str] | None = None) -> None:
    args = silence_argv(argv)
    holder: list[Path] = [Path("genos-debug/verdict/pr-watch/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
