"""Start a proof window on an explicit compositor and stop it with quit or SIGTERM."""

from __future__ import annotations

import os
import signal
import subprocess
from pathlib import Path

from lib.contract import Broken, require_display
from lib.gpu import default_lock

CAPTURE_UI_FALSE = {"ui": False, "name": "pose"}


def lighting_argv(
    binary: str,
    display: str,
    scene: str,
    checkpoint: str,
    port: int,
    out: Path,
    size: str = "320x180",
) -> list[str]:
    require_display(display)
    return [
        binary,
        "--headless",
        "--display",
        display,
        "--scene",
        scene,
        "--checkpoint",
        checkpoint,
        "--mcp-port",
        str(port),
        "--out",
        str(out),
        "--size",
        size,
        "--menu",
        "hidden",
    ]


def stress_argv(binary: str, display: str, frames: int, out: Path) -> list[str]:
    require_display(display)
    return [
        binary,
        "--proof",
        "--display",
        display,
        "--no-panel",
        "--size",
        "320x180",
        "--frames",
        str(frames),
        "--fixed-dt",
        "0.0166667",
        "--out",
        str(out),
    ]


def child_env(parent: dict[str, str], display: str) -> dict[str, str]:
    """The child sees only the requested Wayland display, not the session compositor."""
    require_display(display)
    env = dict(parent)
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("DISPLAY", None)
    env["WAYLAND_DISPLAY"] = display
    return env


def hold_lock(path: Path | None = None) -> Path:
    lock = path or default_lock()
    lock.parent.mkdir(parents=True, exist_ok=True)
    lock.write_text(f"{os.getpid()}\n", encoding="utf-8")
    return lock


def release_lock(path: Path | None = None) -> None:
    lock = path or default_lock()
    if not lock.is_file():
        return
    text = lock.read_text(encoding="utf-8").strip()
    if text == str(os.getpid()):
        lock.unlink()


def stop(proc: subprocess.Popen[str], quit_fn, timeout: float) -> None:
    """MCP quit, then SIGTERM and a wait. A process that ignores that stays recorded as a broken check."""
    try:
        quit_fn()
    except Exception:
        pass
    try:
        proc.wait(timeout=timeout)
        return
    except subprocess.TimeoutExpired:
        proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired as err:
        raise Broken("process ignored SIGTERM") from err


def argv_kills_the_process(argv: list[str]) -> bool:
    return "-9" in argv or "kill" in argv
