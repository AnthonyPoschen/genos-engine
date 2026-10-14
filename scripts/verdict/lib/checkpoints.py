"""Blur and FLIP for each catalog checkpoint. Repeats only. No locked reference."""

from __future__ import annotations

import csv
from pathlib import Path

from lib.contract import FAIL, PASS, UNSURE, Broken
from lib.judge import against_line, load_conf, worse


def read_rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise Broken("no checkpoint rows")
    return rows


def score(rows: list[dict[str, str]]) -> tuple[str, str, list[dict[str, str]]]:
    conf = load_conf()
    band = conf["human_band"]
    blur_line = conf["fail.blur_rel"]
    flip_line = conf["fail.flip_mean"]
    grouped: dict[tuple[str, str], list[dict[str, str]]] = {}
    for row in rows:
        grouped.setdefault((row["scene"], row["checkpoint"]), []).append(row)
    scored: list[dict[str, str]] = []
    overall = PASS
    for (scene, checkpoint), group in grouped.items():
        blurs = [float(item["blur_rel"]) for item in group]
        flips = [float(item["flip_mean"]) for item in group]
        blur_spread = max(blurs) - min(blurs)
        flip_spread = max(flips) - min(flips)
        if len(group) < 2:
            status = UNSURE
            detail = "one sample"
        else:
            status = PASS
            for value, line, name in (
                (max(blurs), blur_line, "blur"),
                (max(flips), flip_line, "flip"),
            ):
                status = worse(status, against_line(value, line, band))
            detail = f"blur={max(blurs):.4g} flip={max(flips):.4g}"
        overall = worse(overall, status)
        scored.append(
            {
                "scene": scene,
                "checkpoint": checkpoint,
                "status": status,
                "detail": detail,
                "blur_spread": f"{blur_spread:.6g}",
                "flip_spread": f"{flip_spread:.6g}",
                "repeats": str(len(group)),
            }
        )
    failing = [row for row in scored if row["status"] == FAIL]
    unsure = [row for row in scored if row["status"] == UNSURE]
    if failing:
        reason = f"{failing[0]['scene']}/{failing[0]['checkpoint']} {failing[0]['detail']}"
    elif unsure:
        reason = f"{unsure[0]['scene']}/{unsure[0]['checkpoint']} {unsure[0]['detail']}"
    else:
        reason = f"{len(scored)} checkpoints within band"
    return overall, reason, scored
