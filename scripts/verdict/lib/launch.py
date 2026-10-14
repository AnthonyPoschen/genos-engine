"""Start a proof window on an explicit compositor and stop it with quit or SIGTERM."""

from __future__ import annotations

import os
import signal
import subprocess
from pathlib import Path

from lib.contract import Broken, absolute_path, require_display
from lib.gpu import default_lock

CAPTURE_UI_FALSE = {"ui": False, "name": "pose"}

_DISPLAY_SUPPORT: dict[str, bool] = {}


def help_lists_display(text: str) -> bool:
    """True when help text documents the --display flag as its own argument."""
    for token in text.split():
        if token == "--display" or token.startswith("--display="):
            return True
    return False


def accepts_display_flag(binary: str) -> bool:
    """Probe `binary --help`. Older genos-lighting builds reject --display."""
    path = Path(binary)
    try:
        stamp = path.stat().st_mtime_ns
    except OSError:
        stamp = 0
    key = f"{path}:{stamp}"
    cached = _DISPLAY_SUPPORT.get(key)
    if cached is not None:
        return cached
    env = dict(os.environ)
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("DISPLAY", None)
    try:
        proc = subprocess.run(
            [binary, "--help"],
            check=False,
            text=True,
            capture_output=True,
            timeout=15,
            env=env,
        )
    except subprocess.TimeoutExpired as err:
        raise Broken(f"{binary} --help timed out") from err
    except OSError as err:
        raise Broken(f"cannot probe {binary}: {err}") from err
    found = help_lists_display((proc.stdout or "") + "\n" + (proc.stderr or ""))
    _DISPLAY_SUPPORT[key] = found
    return found


def _argv_with_display(binary: str, display: str, argv: list[str]) -> list[str]:
    """Pass --display only when this binary documents it.

    The caller still sets WAYLAND_DISPLAY. An older binary places the window
    from that environment and rejects the flag.
    """
    require_display(display)
    if not accepts_display_flag(binary):
        return argv
    return [argv[0], "--display", display, *argv[1:]]


def lighting_argv(
    binary: str,
    display: str,
    scene: str,
    checkpoint: str,
    port: int,
    out: Path,
    size: str = "320x180",
) -> list[str]:
    out = absolute_path(out)
    argv = [
        binary,
        "--headless",
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
    return _argv_with_display(binary, display, argv)


def stress_argv(binary: str, display: str, frames: int, out: Path) -> list[str]:
    out = absolute_path(out)
    argv = [
        binary,
        "--proof",
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
    return _argv_with_display(binary, display, argv)


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
