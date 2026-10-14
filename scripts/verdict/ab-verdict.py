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
from lib.catalog import available
from lib.cache import cache_root, store
from lib.contract import (
    Broken,
    Verdict,
    absolute_path,
    add_common,
    apply_common_mirrors,
    capture_file,
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
from lib.launch import child_env, hold_lock, lighting_launch, release_lock, stop
from lib.mcp import post, quit, wait_ready
from lib.noise import SETTLE_FRAMES, summarize, write_repeats, write_spread
import noise_floor
import precheck


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="A/B verdict against the noise floor.")
    parser.add_argument("base", nargs="?", help="base commit")
    parser.add_argument("head", nargs="?", help="head commit")
    parser.add_argument("--noise-floor", help="spread.json from noise-floor. Omit it to measure BASE first")
    parser.add_argument("--repeats", type=int, help="repeats when measuring the noise floor (default 3)")
    parser.add_argument("--scene", help="scene for the measured noise floor (default stress)")
    parser.add_argument("--checkpoint", help="checkpoint for the measured noise floor (default hall)")
    parser.add_argument("--settle-frames", type=int, help=f"settle cap for the measured noise floor (default {SETTLE_FRAMES})")
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
    ns.scene = mirror(ns.scene, "GENOS_VERDICT_SCENE")
    ns.checkpoint = mirror(ns.checkpoint, "GENOS_VERDICT_CHECKPOINT")
    if ns.noise_floor:
        ns.noise_floor = str(absolute_path(ns.noise_floor))
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
        floor_job(ns)


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
        pre = _precheck(ns, sha, repo)
        if pre.status != "PASS" or pre.reason.startswith("cached "):
            return Verdict(pre.status, pre.reason, pre.evidence, pre.code)
        status, reason, floor_path, rows = _live(ns, repo, evidence)
        if status != "PASS":
            return Verdict(status, reason, evidence)
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
        digest = input_hash(sha, _cache_keys(ns), ns.input)
        store(cache_root(ns.cache_dir), sha, digest, status, reason, evidence)
    return Verdict(status, reason, evidence)


def floor_job(ns: argparse.Namespace) -> tuple[str, str, int, int]:
    """Scene, checkpoint, repeats, and settle cap for a noise floor measured on BASE."""
    if (ns.scene is None) != (ns.checkpoint is None):
        raise Broken("pass both --scene and --checkpoint for the measured noise floor")
    view = available()[0]
    scene = ns.scene or view.lighting_scene
    checkpoint = ns.checkpoint or view.lighting_checkpoint
    repeats = 3 if ns.repeats is None else ns.repeats
    if repeats < 2:
        raise Broken("measured noise floor needs at least 2 repeats")
    settle = SETTLE_FRAMES if ns.settle_frames is None else ns.settle_frames
    if settle < 1:
        raise Broken("settle frames must be at least 1")
    return scene, checkpoint, repeats, settle


def _cache_keys(ns: argparse.Namespace) -> list[str]:
    """Same text precheck hashes, so a later precheck skips this comparison."""
    if ns.noise_floor:
        floor = ns.noise_floor
        repeats = ""
    else:
        _scene, _checkpoint, repeats_n, _settle = floor_job(ns)
        floor = "measured"
        repeats = str(repeats_n)
    return [f"ab:{ns.base}", f"ab:{ns.head}", f"floor:{floor}", f"repeats:{repeats}", *ns.key]


def _precheck(ns: argparse.Namespace, sha: str, repo: Path) -> Verdict:
    argv = [
        "--base",
        ns.base,
        "--head",
        sha,
        "--sha",
        sha,
        "--repo",
        str(repo),
        "--display",
        ns.display,
    ]
    for key in _cache_keys(ns):
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


def _live(
    ns: argparse.Namespace,
    repo: Path,
    evidence: Path,
    build=None,
    measure=None,
    capture_views=None,
) -> tuple[str, str, Path, list[dict[str, str]]]:
    """Build BASE, measure its noise floor when none was given, then build HEAD and capture."""
    build = build or _build
    measure = measure or _measure_base_floor
    capture_views = capture_views or _capture_views
    base_sha = rev_parse(repo, ns.base)
    head_sha = rev_parse(repo, ns.head)
    base_bin = build(repo, base_sha, evidence, "base", ns.binary_name)
    if ns.noise_floor:
        floor_path = absolute_path(ns.noise_floor)
    else:
        status, reason, floor_path = measure(ns, base_bin, evidence)
        if status != "PASS":
            return status, reason, floor_path, []
    head_bin = build(repo, head_sha, evidence, "head", ns.binary_name)
    rows = capture_views(ns, {"base": base_bin, "head": head_bin}, evidence)
    return "PASS", "", floor_path, rows


def _build(repo: Path, sha: str, evidence: Path, label: str, binary_name: str) -> Path:
    import subprocess

    dest = evidence / "work" / label
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
        [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "genos-lighting",
            "--manifest-path",
            str(dest / "Cargo.toml"),
        ],
        cwd=dest,
        check=False,
        text=True,
        capture_output=True,
    )
    (evidence / f"build-{label}.log").write_text(build.stdout + build.stderr, encoding="utf-8")
    if build.returncode != 0:
        raise Broken(f"build {label} failed")
    return dest / "target" / "release" / binary_name


def _measure_base_floor(ns: argparse.Namespace, binary: Path, evidence: Path) -> tuple[str, str, Path]:
    """Run noise-floor on the BASE binary before HEAD is built."""
    scene, checkpoint, repeats, settle = floor_job(ns)
    floor_dir = evidence / "noise-floor"
    floor_dir.mkdir(parents=True, exist_ok=True)
    child = argparse.Namespace(
        binary=str(binary),
        display=ns.display,
        scene=scene,
        checkpoint=checkpoint,
        repeats=repeats,
        settle_frames=settle,
    )
    rows = noise_floor._live(child, floor_dir)
    status, reason, payload = summarize(rows, settle)
    write_repeats(floor_dir / "repeats.csv", rows)
    write_spread(floor_dir / "spread.json", payload)
    (floor_dir / "source.txt").write_text(
        f"base {ns.base}\nscene {scene}\ncheckpoint {checkpoint}\nrepeats {repeats}\n",
        encoding="utf-8",
    )
    return status, f"base noise floor {reason}", floor_dir / "spread.json"


def _capture_views(
    ns: argparse.Namespace,
    built: dict[str, Path],
    evidence: Path,
) -> list[dict[str, str]]:
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
                shots[label] = capture_file(run_dir)
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
    argv, extra = lighting_launch(str(binary), display, scene, checkpoint, port, run_dir)
    (run_dir / "argv.txt").write_text("\n".join(argv) + "\n", encoding="utf-8")
    env_lines = [f"WAYLAND_DISPLAY={display}"]
    env_lines.extend(f"{key}={value}" for key, value in sorted(extra.items()))
    (run_dir / "launch-env.txt").write_text("\n".join(env_lines) + "\n", encoding="utf-8")
    (run_dir / "capture.json").write_text('{"ui": false, "name": "pose"}\n', encoding="utf-8")
    proc = subprocess.Popen(
        argv,
        env=child_env(dict(**__import__("os").environ), display, extra),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        wait_ready(port, proc)
        settled = post(port, "wait_settled", {"timeout_frames": 180, "quiet_frames": 3})
        timing = post(port, "timing", {"frames": 30})
        shot = capture_file(run_dir)
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
    holder: list[Path] = [absolute_path("genos-debug/verdict/ab/unknown")]

    def body() -> Verdict:
        ns = parse(args)
        if ns.out:
            holder[0] = Path(ns.out)
        validate(ns)
        return decide(ns, args, holder)

    run_main(holder, body)


if __name__ == "__main__":
    main()
