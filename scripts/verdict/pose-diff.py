#!/usr/bin/env python3
"""Fixed-pose image diff using capture ui=false.

The previous capture is the comparison, not a locked golden. The first capture
is UNSURE. Status: probation.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.contract import (
    Broken,
    Verdict,
    add_common,
    apply_common_mirrors,
    dry_verdict,
    evidence_dir,
    mirror,
    require_display,
    run_main,
    silence_argv,
    write_args,
)
from lib.gitcheck import fetch_origin
from lib.images import blur_relative, diff_image, flip_mean_p99, load_image, mean_abs, write_png
from lib.judge import against_line, load_conf, worse


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Fixed-pose image diff.")
    parser.add_argument("--scene", help="debug scene")
    parser.add_argument("--checkpoint", help="checkpoint")
    parser.add_argument("--before", help="previous pose image")
    parser.add_argument("--after", help="new pose image")
    parser.add_argument("--fixture", help="directory with before.png and after.png")
    add_common(parser)
    return parser


def parse(argv: list[str]) -> argparse.Namespace:
    ns = build_parser().parse_args(argv)
    apply_common_mirrors(ns)
    ns.scene = mirror(ns.scene, "GENOS_VERDICT_SCENE")
    ns.checkpoint = mirror(ns.checkpoint, "GENOS_VERDICT_CHECKPOINT")
    return ns


def validate(ns: argparse.Namespace) -> None:
    if ns.display:
        require_display(ns.display)
    if ns.dry_run:
        if not ns.scene or not ns.checkpoint:
            raise Broken("missing --scene or --checkpoint")
        require_display(ns.display)
        return
    if ns.fixture or (ns.before and ns.after):
        return
    raise Broken("pass --before and --after, or --fixture")


def decide(ns: argparse.Namespace, argv: list[str], holder: list[Path]) -> Verdict:
    from lib.contract import repo_root

    if not ns.dry_run:
        fetch_origin(repo_root())
    label = ns.sha or (ns.checkpoint or "pose")
    evidence = evidence_dir("pose", label, ns.out)
    evidence.mkdir(parents=True, exist_ok=True)
    holder[0] = evidence
    (evidence / "capture.json").write_text('{"ui": false, "name": "pose"}\n', encoding="utf-8")
    if ns.dry_run:
        return dry_verdict(evidence, argv)
    before, after = _pair(ns)
    if before is None:
        write_png(evidence / "pose.png", load_image(after))
        return Verdict("UNSURE", "no previous pose", evidence)
    left = load_image(before)
    right = load_image(after)
    abs_error = mean_abs(left, right)
    flip = flip_mean_p99(left, right)[0]
    blur = blur_relative(left, right)
    write_png(evidence / "pose.png", right)
    write_png(evidence / "diff.png", diff_image(left, right))
    (evidence / "numbers.csv").write_text(
        "mean_abs,flip_mean,blur_rel\n" + f"{abs_error:.6g},{flip:.6g},{blur:.6g}\n",
        encoding="utf-8",
    )
    write_args(evidence, argv)
    conf = load_conf()
    status = against_line(blur, conf["fail.blur_rel"], conf["human_band"])
    status = worse(status, against_line(flip, conf["fail.flip_mean"], conf["human_band"]))
    reason = f"mean_abs={abs_error:.4g} flip={flip:.4g} blur={blur:.4g}"
    return Verdict(status, reason, evidence)


def _pair(ns: argparse.Namespace) -> tuple[Path | None, Path]:
    if ns.fixture:
        root = Path(ns.fixture)
        after = root / "after.png"
        before = root / "before.png"
        if not after.is_file():
            raise Broken("fixture missing after.png")
        return (before if before.is_file() else None), after
    if not ns.after:
        raise Broken("missing --after")
    after = Path(ns.after)
    if not after.is_file():
        raise Broken(f"missing {after}")
    if ns.before:
        before = Path(ns.before)
        if not before.is_file():
            raise Broken(f"missing {before}")
        return before, after
    return None, after


def main(argv: list[str] | None = None) -> None:
    args = silence_argv(argv)
    holder: list[Path] = [Path("genos-debug/verdict/pose/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
