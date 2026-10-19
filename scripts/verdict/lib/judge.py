"""Thresholds from the lighting reference config, applied to a measured spread."""

from __future__ import annotations

from pathlib import Path

from lib.contract import FAIL, PASS, UNSURE, Broken, repo_root

RANK = {PASS: 0, UNSURE: 1, FAIL: 2}


def worse(left: str, right: str) -> str:
    return left if RANK[left] >= RANK[right] else right


def load_conf(path: Path | None = None) -> dict[str, float]:
    file = path or (repo_root() / "crates" / "lighting-ref" / "lighting-ref.conf")
    if not file.is_file():
        raise Broken(f"missing thresholds {file}")
    values: dict[str, float] = {}
    for line in file.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line or "=" not in line:
            continue
        key, raw = line.split("=", 1)
        values[key.strip()] = float(raw.strip())
    for key in ("human_band", "noise_multiple", "fail.blur_rel", "fail.flip_mean"):
        if key not in values:
            raise Broken(f"thresholds missing {key}")
    return values


def against_noise(value: float, spread: float, multiple: float) -> str:
    """PASS at or under the measured spread. UNSURE until `multiple` times that. Then FAIL.

    A spread of 0 has no floor: a faster or equal value passes, and any increase is UNSURE.
    """
    if value <= spread:
        return PASS
    if spread <= 0 or value < spread * multiple:
        return UNSURE
    return FAIL


def against_line(value: float, fail_line: float, human_band: float) -> str:
    """Inside the band around a fail line is UNSURE. Below the band passes. Above it fails."""
    low = fail_line * (1.0 - human_band)
    high = fail_line * (1.0 + human_band)
    if value <= low:
        return PASS
    if value >= high:
        return FAIL
    return UNSURE
