"""Per-view frame-time and picture deltas against a measured noise floor."""

from __future__ import annotations

import csv
import json
from pathlib import Path

from lib.contract import FAIL, PASS, UNSURE, Broken
from lib.judge import against_noise, load_conf, worse


def load_floor(path: Path) -> dict:
    if not path.is_file():
        raise Broken(f"missing noise floor {path}")
    data = json.loads(path.read_text(encoding="utf-8"))
    frame = data.get("frame_ms") or {}
    if "spread" not in frame:
        raise Broken("noise floor has no frame_ms spread")
    return data


def read_views(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise Broken("no views")
    return rows


def score_views(
    rows: list[dict[str, str]],
    floor: dict,
    multiple: float | None = None,
) -> tuple[str, str, list[dict[str, str]], list[dict[str, str]]]:
    conf = load_conf()
    factor = multiple if multiple is not None else conf["noise_multiple"]
    frame_spread = float(floor["frame_ms"]["spread"])
    picture_floor = {
        "mean_abs": _spread(floor.get("mean_abs")),
        "flip_mean": _spread(floor.get("flip_mean")),
        "blur_rel": _spread(floor.get("blur_rel")),
    }
    scored: list[dict[str, str]] = []
    by_scene: dict[str, str] = {}
    for row in rows:
        view_status, detail = _one(row, frame_spread, picture_floor, factor)
        scene = row["scene"]
        by_scene[scene] = worse(by_scene.get(scene, PASS), view_status)
        scored.append(
            {
                "scene": scene,
                "checkpoint": row["checkpoint"],
                "status": view_status,
                "detail": detail,
                "delta_ms": detail.split(" ")[0],
            }
        )
    scene_rows = [{"scene": scene, "status": status} for scene, status in by_scene.items()]
    overall = PASS
    for status in by_scene.values():
        overall = worse(overall, status)
    failing = [row for row in scored if row["status"] == FAIL]
    unsure = [row for row in scored if row["status"] == UNSURE]
    if failing:
        reason = f"{failing[0]['scene']}/{failing[0]['checkpoint']} {failing[0]['detail']}"
    elif unsure:
        reason = f"{unsure[0]['scene']}/{unsure[0]['checkpoint']} {unsure[0]['detail']}"
    else:
        reason = f"{len(by_scene)} scenes within noise"
    return overall, reason, scored, scene_rows


def _spread(node) -> float | None:
    if not isinstance(node, dict) or "spread" not in node:
        return None
    return float(node["spread"])


def _one(row: dict[str, str], frame_spread: float, picture_floor: dict[str, float | None], multiple: float) -> tuple[str, str]:
    base = float(row["base_ms"])
    head = float(row["head_ms"])
    delta = head - base
    status = against_noise(delta, frame_spread, multiple)
    detail = f"delta_ms={delta:.4g}"
    saw_picture = False
    for key in ("mean_abs", "flip_mean", "blur_rel"):
        raw = (row.get(key) or "").strip()
        if not raw:
            continue
        saw_picture = True
        spread = picture_floor[key]
        if spread is None:
            status = worse(status, UNSURE)
            detail += f" {key}=no-floor"
            continue
        picture = against_noise(float(raw), spread, multiple)
        status = worse(status, picture)
        detail += f" {key}={raw}"
    if not saw_picture:
        status = worse(status, UNSURE)
        detail += " no picture"
    return status, detail
