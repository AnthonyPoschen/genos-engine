#!/usr/bin/env python3
"""N repeats of one unchanged commit. Records the spread and the settle cap.

A cold start that is still unsettled after --settle-frames (default 180) fails.
Status: probation. A miss becomes a test in scripts/verdict/tests/test_misses.py.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.cache import cache_root, store
from lib.contract import (
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
from lib.gitcheck import head_sha
from lib.launch import child_env, hold_lock, lighting_argv, release_lock, stop
from lib.mcp import post, quit, wait_ready
from lib.noise import SETTLE_FRAMES, read_repeats, summarize, write_repeats, write_spread
import precheck


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Measure the noise floor of one commit.")
    parser.add_argument("--repeats", type=int, help="how many cold starts")
    parser.add_argument("--settle-frames", type=int, help=f"fail past this frame (default {SETTLE_FRAMES})")
    parser.add_argument("--scene", help="debug scene name")
    parser.add_argument("--checkpoint", help="checkpoint in that scene")
    parser.add_argument("--fixture", help="directory with repeats.csv (and optional images)")
    parser.add_argument("--base", help="base ref for the precheck merge")
    parser.add_argument("--binary", help="genos-lighting binary for a live run")
    parser.add_argument("--repo", help="repository (default: this checkout)")
    add_common(parser)
    return parser


def parse(argv: list[str]) -> argparse.Namespace:
    ns = build_parser().parse_args(argv)
    apply_common_mirrors(ns)
    ns.base = mirror(ns.base, "GENOS_VERDICT_BASE")
    ns.scene = mirror(ns.scene, "GENOS_VERDICT_SCENE")
    ns.checkpoint = mirror(ns.checkpoint, "GENOS_VERDICT_CHECKPOINT")
    if ns.settle_frames is None:
        raw = mirror(None, "GENOS_VERDICT_SETTLE_FRAMES")
        ns.settle_frames = int(raw) if raw is not None else SETTLE_FRAMES
    return ns


def validate(ns: argparse.Namespace) -> None:
    if ns.repeats is None or ns.repeats < 1:
        raise Broken("missing --repeats")
    if ns.settle_frames < 1:
        raise Broken("settle frames must be at least 1")
    if ns.fixture:
        return
    if not ns.base:
        raise Broken("missing --base")
    if not ns.scene or not ns.checkpoint:
        raise Broken("missing --scene or --checkpoint")
    require_display(ns.display)


def resolve_rows(rows: list[dict[str, str]], root: Path) -> list[dict[str, str]]:
    resolved: list[dict[str, str]] = []
    for row in rows:
        item = dict(row)
        name = (item.get("image") or "").strip()
        if name and not Path(name).is_absolute():
            item["image"] = str(root / name)
        resolved.append(item)
    return resolved


def decide(ns: argparse.Namespace, argv: list[str], holder: list[Path]) -> Verdict:
    from lib.contract import repo_root

    repo = Path(ns.repo) if ns.repo else repo_root()
    if ns.dry_run:
        label = ns.sha or "dry-run"
        evidence = evidence_dir("noise-floor", label, ns.out)
        evidence.mkdir(parents=True, exist_ok=True)
        holder[0] = evidence
        note = evidence / "capture.json"
        note.write_text('{"ui": false, "name": "pose"}\n', encoding="utf-8")
        return dry_verdict(evidence, argv)

    sha = ns.sha if ns.fixture and ns.sha else head_sha(repo, ns.sha)
    evidence = evidence_dir("noise-floor", sha, ns.out)
    evidence.mkdir(parents=True, exist_ok=True)
    holder[0] = evidence
    write_args(evidence, argv)
    (evidence / "capture.json").write_text(
        '{"ui": false, "name": "pose"}\n',
        encoding="utf-8",
    )

    if not ns.fixture:
        pre = _precheck(ns, sha, repo)
        if pre.status != "PASS" or pre.reason.startswith("cached "):
            return Verdict(pre.status, pre.reason, pre.evidence, pre.code)

    if ns.fixture:
        rows = resolve_rows(read_repeats(Path(ns.fixture) / "repeats.csv"), Path(ns.fixture))
    else:
        rows = _live(ns, evidence)
    if len(rows) != ns.repeats and ns.fixture:
        raise Broken(f"fixture has {len(rows)} repeats, --repeats is {ns.repeats}")
    status, reason, payload = summarize(rows, ns.settle_frames)
    write_repeats(evidence / "repeats.csv", rows)
    write_spread(evidence / "spread.json", payload)
    _numbers(evidence / "numbers.csv", rows, payload)
    if status == "PASS" or status == "FAIL" or status == "UNSURE":
        if not ns.fixture:
            digest = input_hash(sha, list(ns.key), ns.input)
            if status != "UNSURE":
                store(cache_root(ns.cache_dir), sha, digest, status, reason, evidence)
        return Verdict(status, reason, evidence)
    raise Broken(f"unknown status {status}")


def _precheck(ns: argparse.Namespace, sha: str, repo: Path) -> Verdict:
    argv = [
        "--base",
        ns.base,
        "--sha",
        sha,
        "--repo",
        str(repo),
        "--display",
        ns.display,
    ]
    for key in ns.key:
        argv.extend(["--key", key])
    for item in ns.input:
        argv.extend(["--input", item])
    if ns.cache_dir:
        argv.extend(["--cache-dir", ns.cache_dir])
    if ns.out:
        argv.extend(["--out", str(Path(ns.out) / "precheck")])
    parsed = precheck.parse(argv)
    precheck.validate(parsed)
    return precheck.decide(parsed, argv, [Path("genos-debug/verdict/precheck/unknown")])


def _live(ns: argparse.Namespace, evidence: Path) -> list[dict[str, str]]:
    import socket
    import subprocess

    binary = ns.binary or "target/release/genos-lighting"
    if not Path(binary).is_file():
        raise Broken(f"missing binary {binary}")
    display = require_display(ns.display)
    lock = hold_lock()
    try:
        rows: list[dict[str, str]] = []
        for index in range(1, ns.repeats + 1):
            run_dir = evidence / f"run-{index}"
            run_dir.mkdir(parents=True, exist_ok=True)
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            argv = lighting_argv(binary, display, ns.scene, ns.checkpoint, port, run_dir)
            (run_dir / "argv.txt").write_text("\n".join(argv) + "\n", encoding="utf-8")
            (run_dir / "capture.json").write_text('{"ui": false, "name": "pose"}\n', encoding="utf-8")
            proc = subprocess.Popen(
                argv,
                env=child_env(os_environ(), display),
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                row = _measure(proc, port, ns.settle_frames, run_dir, index)
            finally:
                stop(proc, lambda port=port: quit(port), 20)
            rows.append(row)
        return rows
    finally:
        release_lock(lock)


def os_environ() -> dict[str, str]:
    import os

    return dict(os.environ)


def _measure(proc, port: int, settle_limit: int, run_dir: Path, index: int) -> dict[str, str]:
    wait_ready(port, proc)
    settled = post(
        port,
        "wait_settled",
        {"timeout_frames": settle_limit, "quiet_frames": 3},
    )
    timing = post(port, "timing", {"frames": 30})
    shot = run_dir / "pose.png"
    post(port, "capture", {"ui": False, "name": "pose", "path": str(shot)})
    log = run_dir / "mcp.json"
    log.write_text(
        __import__("json").dumps({"settled": settled, "timing": timing}) + "\n",
        encoding="utf-8",
    )
    frame_ms = timing.get("frame_ms")
    if not isinstance(frame_ms, (int, float)):
        raise Broken("timing reply has no frame_ms")
    return {
        "repeat": str(index),
        "frame_ms": str(frame_ms),
        "settled": "1" if settled.get("settled") is True else "0",
        "settle_frames": str(int(settled.get("frames") or settle_limit)),
        "image": str(shot),
    }


def _numbers(path: Path, rows: list[dict[str, str]], payload: dict) -> None:
    lines = ["repeat,frame_ms,settled,settle_frames"]
    for row in rows:
        lines.append(
            f"{row['repeat']},{row['frame_ms']},{row['settled']},{row['settle_frames']}"
        )
    frame = payload.get("frame_ms") or {}
    lines.append(f"# spread,{frame.get('spread', '')}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> None:
    args = silence_argv(argv)
    holder: list[Path] = [Path("genos-debug/verdict/noise-floor/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
