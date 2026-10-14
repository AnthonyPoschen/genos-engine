#!/usr/bin/env python3
"""Blur and FLIP for each catalog checkpoint.

UNSURE inside the threshold band. The repeat spread is saved.
Reference images are not written. Status: probation.
"""

from __future__ import annotations

import csv
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.checkpoints import read_rows, score
from lib.contract import (
    Broken,
    Verdict,
    add_common,
    apply_common_mirrors,
    dry_verdict,
    evidence_dir,
    require_display,
    run_main,
    silence_argv,
    write_args,
)


def build_parser() -> argparse.ArgumentParser:
    import argparse

    parser = argparse.ArgumentParser(description="Checkpoint catalog blur/FLIP verdict.")
    parser.add_argument("--fixture", help="directory with repeats.csv")
    parser.add_argument("--repeats", type=int, help="expected repeats per checkpoint")
    add_common(parser)
    return parser


def parse(argv: list[str]):
    ns = build_parser().parse_args(argv)
    apply_common_mirrors(ns)
    return ns


def validate(ns) -> None:
    if ns.repeats is not None and ns.repeats < 1:
        raise Broken("--repeats must be at least 1")
    if ns.dry_run or ns.fixture:
        if ns.display:
            require_display(ns.display)
        return
    raise Broken("pass --fixture; this runner does not lock reference images")


def decide(ns, argv: list[str], holder: list[Path]) -> Verdict:
    label = ns.sha or "fixture"
    evidence = evidence_dir("checkpoint", label, ns.out)
    evidence.mkdir(parents=True, exist_ok=True)
    holder[0] = evidence
    if ns.dry_run:
        (evidence / "references.txt").write_text("reference images are not locked\n", encoding="utf-8")
        return dry_verdict(evidence, argv)
    if not ns.fixture:
        raise Broken("pass --fixture")
    rows = read_rows(Path(ns.fixture) / "repeats.csv")
    if ns.repeats is not None:
        counts: dict[tuple[str, str], int] = {}
        for row in rows:
            key = (row["scene"], row["checkpoint"])
            counts[key] = counts.get(key, 0) + 1
        for key, count in counts.items():
            if count != ns.repeats:
                raise Broken(f"{key[0]}/{key[1]} has {count} repeats, expected {ns.repeats}")
    status, reason, scored = score(rows)
    write_args(evidence, argv)
    with (evidence / "checkpoints.csv").open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=["scene", "checkpoint", "status", "detail", "blur_spread", "flip_spread", "repeats"],
        )
        writer.writeheader()
        writer.writerows(scored)
    (evidence / "numbers.csv").write_text(
        Path(ns.fixture).joinpath("repeats.csv").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    (evidence / "references.txt").write_text("reference images are not locked\n", encoding="utf-8")
    forbidden = evidence / "reference"
    if forbidden.exists():
        raise Broken("refused to keep a reference image")
    return Verdict(status, reason, evidence)


def main(argv: list[str] | None = None) -> None:
    args = silence_argv(argv)
    holder: list[Path] = [Path("genos-debug/verdict/checkpoint/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
