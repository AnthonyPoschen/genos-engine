#!/usr/bin/env python3
"""Compare BASE and HEAD against a measured noise floor.

PASS when no view is slower beyond the noise and the pictures match.
One scene line is written to scenes.csv. Stdout is the overall verdict.
Status: probation.
"""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.ab import load_floor, read_views, score_views
from lib.cache import cache_root, store
from lib.catalog import available
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
from lib.gitcheck import head_sha, rev_parse
from lib.images import diff_image, flip_mean_p99, load_image, mean_abs, write_png
from lib.images import blur_relative
from lib.launch import child_env, hold_lock, lighting_argv, release_lock, stop
from lib.mcp import post, quit, wait_ready


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="A/B verdict against the noise floor.")
    parser.add_argument("base", nargs="?", help="base commit")
    parser.add_argument("head", nargs="?", help="head commit")
    parser.add_argument("--noise-floor", help="spread.json from noise-floor")
    parser.add_argument("--fixture", help="directory with views.csv and the noise floor")
    parser.add_argument("--live", action="store_true", help="build both commits and capture")
    parser.add_argument("--repo", help="repository (default: this checkout)")
    parser.add_argument("--binary-name", default="genos-lighting", help="binary built in each worktree")
    add_common(parser)
    return parser


def parse(argv: list[str]) -> argparse.Namespace:
    ns = build_parser().parse_args(argv)
    apply_common_mirrors(ns)
    ns.base = mirror(ns.base, "GENOS_VERDICT_BASE")
    ns.head = mirror(ns.head, "GENOS_VERDICT_HEAD")
    ns.noise_floor = mirror(ns.noise_floor, "GENOS_VERDICT_NOISE_FLOOR")
    return ns


def validate(ns: argparse.Namespace) -> None:
    if not ns.base or not ns.head:
        raise Broken("missing BASE or HEAD")
    if ns.display:
        require_display(ns.display)
    if ns.dry_run or ns.fixture:
        return
    if not ns.live:
        raise Broken("pass --fixture or --live")
    require_display(ns.display)
    if not ns.noise_floor:
        raise Broken("missing --noise-floor")


def decide(ns: argparse.Namespace, argv: list[str], holder: list[Path]) -> Verdict:
    from lib.contract import repo_root

    repo = Path(ns.repo) if ns.repo else repo_root()
    label = ns.sha or ns.head or "dry-run"
    if ns.dry_run:
        evidence = evidence_dir("ab", label, ns.out)
        evidence.mkdir(parents=True, exist_ok=True)
        holder[0] = evidence
        return dry_verdict(evidence, argv)
    if ns.fixture:
        sha = ns.sha or "fixture"
    else:
        sha = head_sha(repo, ns.sha or ns.head)
    evidence = evidence_dir("ab", sha, ns.out)
    evidence.mkdir(parents=True, exist_ok=True)
    holder[0] = evidence
    write_args(evidence, argv)
    if ns.fixture:
        root = Path(ns.fixture)
        floor_path = Path(ns.noise_floor) if ns.noise_floor else root / "spread.json"
        rows = read_views(root / "views.csv")
        _diff_fixtures(root, rows, evidence)
    else:
        floor_path = Path(ns.noise_floor)
        rows = _live(ns, repo, evidence)
    floor = load_floor(floor_path)
    status, reason, views, scenes = score_views(rows, floor)
    _write_csv(evidence / "views.csv", views, ["scene", "checkpoint", "status", "detail", "delta_ms"])
    _write_csv(evidence / "scenes.csv", scenes, ["scene", "status"])
    (evidence / "numbers.csv").write_text(
        "scene,checkpoint,base_ms,head_ms,mean_abs,flip_mean,blur_rel\n"
        + "".join(
            f"{row.get('scene','')},{row.get('checkpoint','')},{row.get('base_ms','')},{row.get('head_ms','')},{row.get('mean_abs','')},{row.get('flip_mean','')},{row.get('blur_rel','')}\n"
            for row in rows
        ),
        encoding="utf-8",
    )
    if not ns.fixture and status != "UNSURE":
        digest = input_hash(sha, [ns.base, ns.head, *ns.key], ns.input)
        store(cache_root(ns.cache_dir), sha, digest, status, reason, evidence)
    return Verdict(status, reason, evidence)


def _diff_fixtures(root: Path, rows: list[dict[str, str]], evidence: Path) -> None:
    for row in rows:
        base = root / f"{row['scene']}-{row['checkpoint']}-base.png"
        head = root / f"{row['scene']}-{row['checkpoint']}-head.png"
        if not base.is_file() or not head.is_file():
            continue
        left = load_image(base)
        right = load_image(head)
        row["mean_abs"] = f"{mean_abs(left, right):.6g}"
        row["flip_mean"] = f"{flip_mean_p99(left, right)[0]:.6g}"
        row["blur_rel"] = f"{blur_relative(left, right):.6g}"
        dest = evidence / f"{row['scene']}-{row['checkpoint']}-diff.png"
        write_png(dest, diff_image(left, right))


def _write_csv(path: Path, rows: list[dict[str, str]], fields: list[str]) -> None:
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)


def _live(ns: argparse.Namespace, repo: Path, evidence: Path) -> list[dict[str, str]]:
    import subprocess

    base = rev_parse(repo, ns.base)
    head = rev_parse(repo, ns.head)
    built: dict[str, Path] = {}
    work = evidence / "work"
    for label, sha in (("base", base), ("head", head)):
        dest = work / label
        if dest.exists():
            subprocess.run(["git", "worktree", "remove", "--force", str(dest)], cwd=repo, check=False)
        proc = subprocess.run(
            ["git", "worktree", "add", "--detach", str(dest), sha],
            cwd=repo,
            check=False,
            text=True,
            capture_output=True,
        )
        if proc.returncode != 0:
            raise Broken((proc.stderr or proc.stdout).strip() or "worktree add failed")
        build = subprocess.run(
            ["cargo", "build", "--release", "-p", "genos-lighting", "--manifest-path", str(dest / "Cargo.toml")],
            cwd=dest,
            check=False,
            text=True,
            capture_output=True,
        )
        (evidence / f"build-{label}.log").write_text(build.stdout + build.stderr, encoding="utf-8")
        if build.returncode != 0:
            raise Broken(f"build {label} failed")
        built[label] = dest / "target" / "release" / ns.binary_name
    display = require_display(ns.display)
    lock = hold_lock()
    rows: list[dict[str, str]] = []
    try:
        for view in available():
            measured: dict[str, dict[str, str]] = {}
            shots: dict[str, Path] = {}
            for label, binary in built.items():
                run_dir = evidence / label / f"{view.scene}-{view.checkpoint}"
                run_dir.mkdir(parents=True, exist_ok=True)
                measured[label] = _capture(binary, display, view.lighting_scene, view.lighting_checkpoint, run_dir)
                shots[label] = run_dir / "pose.png"
            left = load_image(shots["base"])
            right = load_image(shots["head"])
            rows.append(
                {
                    "scene": view.scene,
                    "checkpoint": view.checkpoint,
                    "base_ms": measured["base"]["frame_ms"],
                    "head_ms": measured["head"]["frame_ms"],
                    "mean_abs": f"{mean_abs(left, right):.6g}",
                    "flip_mean": f"{flip_mean_p99(left, right)[0]:.6g}",
                    "blur_rel": f"{blur_relative(left, right):.6g}",
                }
            )
            write_png(evidence / f"{view.scene}-{view.checkpoint}-diff.png", diff_image(left, right))
    finally:
        release_lock(lock)
    return rows


def _capture(binary: Path, display: str, scene: str, checkpoint: str, run_dir: Path) -> dict[str, str]:
    import socket
    import subprocess

    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = int(sock.getsockname()[1])
    argv = lighting_argv(str(binary), display, scene, checkpoint, port, run_dir)
    (run_dir / "argv.txt").write_text("\n".join(argv) + "\n", encoding="utf-8")
    (run_dir / "capture.json").write_text('{"ui": false, "name": "pose"}\n', encoding="utf-8")
    proc = subprocess.Popen(
        argv,
        env=child_env(dict(**__import__("os").environ), display),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        wait_ready(port, proc)
        settled = post(port, "wait_settled", {"timeout_frames": 180, "quiet_frames": 3})
        timing = post(port, "timing", {"frames": 30})
        shot = run_dir / "pose.png"
        post(port, "capture", {"ui": False, "name": "pose", "path": str(shot)})
        if settled.get("settled") is not True:
            raise Broken(f"{scene} {checkpoint} did not settle within 180 frames")
        frame_ms = timing.get("frame_ms")
        if not isinstance(frame_ms, (int, float)):
            raise Broken("timing reply has no frame_ms")
        return {"frame_ms": str(frame_ms)}
    finally:
        stop(proc, lambda port=port: quit(port), 20)


def main(argv: list[str] | None = None) -> None:
    args = silence_argv(argv)
    holder: list[Path] = [Path("genos-debug/verdict/ab/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
