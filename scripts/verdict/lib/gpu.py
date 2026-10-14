"""Busy and lock probe. This does not open a window or submit GPU work."""

from __future__ import annotations

import os
from pathlib import Path

from lib.contract import PASS, UNSURE

LOCK_RELATIVE = Path("genos-debug") / "verdict" / "gpu.lock"
BUSY_PERCENT = 50.0


def default_lock() -> Path:
    return LOCK_RELATIVE


def read_process_file(path: Path) -> list[str]:
    return [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def live_processes() -> list[str]:
    names: list[str] = []
    proc_root = Path("/proc")
    if not proc_root.is_dir():
        return names
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        command = entry / "cmdline"
        try:
            raw = command.read_bytes().replace(b"\0", b" ").decode(errors="replace")
        except OSError:
            continue
        text = raw.strip()
        if text:
            names.append(text)
    return names


def read_util_file(path: Path) -> float | None:
    text = path.read_text(encoding="utf-8").strip()
    if not text or text.lower() == "unknown":
        return None
    return float(text.split()[0])


def live_util() -> float | None:
    samples: list[float] = []
    drm = Path("/sys/class/drm")
    if drm.is_dir():
        for card in drm.glob("card*/device/gpu_busy_percent"):
            try:
                samples.append(float(card.read_text(encoding="utf-8").strip()))
            except (OSError, ValueError):
                continue
    if samples:
        return max(samples)
    nvidia = Path("/usr/bin/nvidia-smi")
    if nvidia.is_file():
        import subprocess

        proc = subprocess.run(
            [
                str(nvidia),
                "--query-gpu=utilization.gpu",
                "--format=csv,noheader,nounits",
            ],
            check=False,
            text=True,
            capture_output=True,
        )
        if proc.returncode == 0:
            for line in proc.stdout.splitlines():
                line = line.strip()
                if line.isdigit():
                    samples.append(float(line))
        if samples:
            return max(samples)
    return None


def lock_holder(path: Path) -> int | None:
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8").strip()
    if not text.isdigit():
        return None
    pid = int(text)
    if pid <= 0:
        return None
    try:
        os.kill(pid, 0)
    except OSError:
        return None
    return pid


def stress_running(commands: list[str]) -> bool:
    for command in commands:
        for token in command.split():
            base = Path(token).name
            if base == "genos-stress" or base.startswith("genos-stress"):
                return True
    return False


def classify(
    commands: list[str],
    util: float | None,
    limit: float,
    lock_pid: int | None,
) -> tuple[str, str]:
    if lock_pid is not None:
        return UNSURE, f"busy lock pid={lock_pid}"
    if stress_running(commands):
        return UNSURE, "busy genos-stress"
    if util is not None and util > limit:
        return UNSURE, f"busy gpu {util:.0f}%"
    if util is None:
        return PASS, "clear gpu unread"
    return PASS, f"clear gpu {util:.0f}%"
