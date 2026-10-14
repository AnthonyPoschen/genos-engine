#!/usr/bin/env python3
"""Cheap checks before any verdict uses the GPU.

Merge conflicts, a busy GPU or genos-stress lock, and a verdict cache hit.
Status: probation. A miss becomes a test in scripts/verdict/tests/test_misses.py.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.cache import cache_root, load, replay
from lib.contract import (
    FAIL,
    PASS,
    Broken,
    Verdict,
    add_common,
    apply_common_mirrors,
    dry_verdict,
    evidence_dir,
    input_hash,
    mirror,
    require_display,
    run_main,
    silence_argv,
    write_args,
)
from lib.gitcheck import head_sha, merge_conflicts, rev_parse
from lib.gpu import (
    BUSY_PERCENT,
    classify,
    default_lock,
    live_processes,
    live_util,
    lock_holder,
    read_process_file,
    read_util_file,
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Verdict precheck. Does not use the GPU.")
    parser.add_argument("--base", help="base ref for the merge-tree check")
    parser.add_argument("--head", help="head ref (default HEAD)")
    parser.add_argument("--repo", help="repository (default: this checkout)")
    parser.add_argument(
        "--gpu-busy-percent",
        type=float,
        help=f"UNSURE busy when GPU utilization is above this (default {BUSY_PERCENT:g})",
    )
    parser.add_argument("--process-file", help="process list fixture, one command per line")
    parser.add_argument("--gpu-util-file", help="GPU utilization fixture, one number")
    parser.add_argument("--lock", help="pid lock file (default ./genos-debug/verdict/gpu.lock)")
    add_common(parser)
    return parser


def parse(argv: list[str]) -> argparse.Namespace:
    parser = build_parser()
    ns = parser.parse_args(argv)
    apply_common_mirrors(ns)
    ns.base = mirror(ns.base, "GENOS_VERDICT_BASE")
    ns.head = mirror(ns.head, "GENOS_VERDICT_HEAD")
    ns.repo = mirror(ns.repo, "GENOS_VERDICT_REPO")
    if ns.gpu_busy_percent is None:
        raw = mirror(None, "GENOS_VERDICT_GPU_BUSY_PERCENT")
        ns.gpu_busy_percent = float(raw) if raw is not None else BUSY_PERCENT
    return ns


def validate(ns: argparse.Namespace) -> None:
    if not ns.base:
        raise Broken("missing --base")
    for raw in ns.input:
        if not Path(raw).is_file():
            raise Broken(f"input missing {raw}")
    if ns.gpu_busy_percent < 0 or ns.gpu_busy_percent > 100:
        raise Broken("gpu busy percent must be from 0 to 100")
    if ns.display:
        require_display(ns.display)


def decide(ns: argparse.Namespace, argv: list[str], holder: list[Path]) -> Verdict:
    from lib.contract import repo_root

    repo = Path(ns.repo) if ns.repo else repo_root()
    if ns.dry_run:
        label = ns.sha or ns.head or "dry-run"
        evidence = evidence_dir("precheck", label, ns.out)
        evidence.mkdir(parents=True, exist_ok=True)
        holder[0] = evidence
        return dry_verdict(evidence, argv)

    sha = head_sha(repo, ns.sha or ns.head)
    evidence = evidence_dir("precheck", sha, ns.out)
    evidence.mkdir(parents=True, exist_ok=True)
    holder[0] = evidence
    write_args(evidence, argv)

    base = rev_parse(repo, ns.base)
    head = rev_parse(repo, ns.head or sha)
    digest = input_hash(head, list(ns.key), ns.input)
    (evidence / "sha.txt").write_text(f"base {base}\nhead {head}\ninput {digest}\n", encoding="utf-8")

    conflict, log = merge_conflicts(repo, base, head)
    (evidence / "merge-tree.log").write_text(log, encoding="utf-8")
    if conflict:
        return Verdict(FAIL, f"merge conflict with {ns.base}", evidence)

    commands = read_process_file(Path(ns.process_file)) if ns.process_file else live_processes()
    util = read_util_file(Path(ns.gpu_util_file)) if ns.gpu_util_file else live_util()
    lock_path = Path(ns.lock) if ns.lock else default_lock()
    pid = lock_holder(lock_path)
    (evidence / "gpu.txt").write_text(
        "commands\n"
        + "\n".join(commands)
        + f"\nutil {util}\nlimit {ns.gpu_busy_percent}\nlock {pid}\n",
        encoding="utf-8",
    )
    status, reason = classify(commands, util, ns.gpu_busy_percent, pid)
    if status != PASS:
        return Verdict(status, reason, evidence)

    record = load(cache_root(ns.cache_dir), head, digest)
    if record is not None:
        (evidence / "skipped.txt").write_text("cache hit\n", encoding="utf-8")
        return replay(record, evidence)
    return Verdict(PASS, reason, evidence)


def main(argv: list[str] | None = None) -> None:
    args = silence_argv(argv)
    holder: list[Path] = [Path("genos-debug/verdict/precheck/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
