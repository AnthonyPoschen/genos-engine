"""Spread across repeats of one unchanged commit, and the cold-start settle cap."""

from __future__ import annotations

import csv
import json
from pathlib import Path

from lib.contract import FAIL, PASS, UNSURE
from lib.images import Image, blur_relative, flip_mean_p99, load_image, mean_abs

SETTLE_FRAMES = 180


def read_repeats(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle))


def write_repeats(path: Path, rows: list[dict[str, str]]) -> None:
    fields = ["repeat", "frame_ms", "settled", "settle_frames", "image"]
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow(row)


def _settled(row: dict[str, str]) -> bool:
    return row.get("settled", "").strip().lower() in {"1", "true", "yes"}


def summarize(rows: list[dict[str, str]], settle_limit: int) -> tuple[str, str, dict]:
    if not rows:
        return UNSURE, "no repeats", {"repeats": 0, "settle_limit": settle_limit}
    frames = [float(row["frame_ms"]) for row in rows]
    spread = max(frames) - min(frames)
    picture = _picture_spread(rows)
    failed = False
    for row in rows:
        count = int(float(row["settle_frames"]))
        if not _settled(row) or count > settle_limit:
            failed = True
            break
    payload = {
        "repeats": len(rows),
        "settle_limit": settle_limit,
        "settle_frames_max": max(int(float(row["settle_frames"])) for row in rows),
        "frame_ms": {
            "min": min(frames),
            "max": max(frames),
            "mean": sum(frames) / len(frames),
            "spread": spread,
        },
        "mean_abs": picture["mean_abs"],
        "flip_mean": picture["flip_mean"],
        "blur_rel": picture["blur_rel"],
    }
    if failed:
        return FAIL, f"cold start did not settle within {settle_limit} frames", payload
    if len(rows) < 2:
        return UNSURE, "one sample", payload
    return PASS, f"spread frame_ms={spread:.4g}", payload


def _picture_spread(rows: list[dict[str, str]]) -> dict[str, dict[str, float] | None]:
    images: list[Image] = []
    for row in rows:
        name = (row.get("image") or "").strip()
        if not name:
            return {"mean_abs": None, "flip_mean": None, "blur_rel": None}
        images.append(load_image(Path(name)))
    if len(images) < 2:
        return {"mean_abs": None, "flip_mean": None, "blur_rel": None}
    abs_values: list[float] = []
    flip_values: list[float] = []
    blur_values: list[float] = []
    first = images[0]
    for other in images[1:]:
        abs_values.append(mean_abs(first, other))
        flip_values.append(flip_mean_p99(first, other)[0])
        blur_values.append(blur_relative(first, other))
    return {
        "mean_abs": {"spread": max(abs_values), "max": max(abs_values), "min": min(abs_values)},
        "flip_mean": {"spread": max(flip_values), "max": max(flip_values), "min": min(flip_values)},
        "blur_rel": {"spread": max(blur_values), "max": max(blur_values), "min": min(blur_values)},
    }


def write_spread(path: Path, payload: dict) -> None:
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
