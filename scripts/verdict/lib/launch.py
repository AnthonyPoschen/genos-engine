"""Start a proof window on an explicit compositor and stop it with quit or SIGTERM."""

from __future__ import annotations

import os
import re
import signal
import subprocess
from pathlib import Path

from lib.contract import Broken, absolute_path, require_display
from lib.gpu import default_lock

CAPTURE_UI_FALSE = {"ui": False, "name": "pose"}

_HELP_TEXT: dict[str, str] = {}


def help_flags(text: str) -> set[str]:
    """Flag names documented in `binary --help`, such as `--mcp-port`."""
    found: set[str] = set()
    for token in text.split():
        if not token.startswith("--") or token == "--":
            continue
        name = token.split("=", 1)[0].rstrip(",.;:")
        if len(name) > 2 and all(ch.isalnum() or ch == "-" for ch in name[2:]):
            found.add(name)
    return found


def help_lists_display(text: str) -> bool:
    """True when help text documents the --display flag as its own argument."""
    return "--display" in help_flags(text)


def binary_help(binary: str) -> str:
    """Stdout and stderr of `binary --help`. The probe does not attach to a compositor."""
    path = Path(binary)
    try:
        stamp = path.stat().st_mtime_ns
    except OSError:
        stamp = 0
    key = f"{path}:{stamp}"
    cached = _HELP_TEXT.get(key)
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
    text = (proc.stdout or "") + "\n" + (proc.stderr or "")
    _HELP_TEXT[key] = text
    return text


def accepts_display_flag(binary: str) -> bool:
    """Probe `binary --help`. Older genos-lighting builds reject --display."""
    return help_lists_display(binary_help(binary))


def _mentioned(text: str, name: str) -> bool:
    return re.search(rf"\b{re.escape(name)}\b", text) is not None


def _flag_or_env(
    text: str,
    argv: list[str],
    env: dict[str, str],
    flag: str,
    value: str | None,
    mirror: str | None,
) -> bool:
    """Pass `flag` when help lists it. Otherwise set `mirror` when help names that variable."""
    if flag in help_flags(text):
        argv.append(flag)
        if value is not None:
            argv.append(value)
        return True
    if mirror and value is not None and _mentioned(text, mirror):
        env[mirror] = value
        return True
    return False


def lighting_launch(
    binary: str,
    display: str,
    scene: str,
    checkpoint: str,
    port: int,
    out: Path,
    size: str = "320x180",
) -> tuple[list[str], dict[str, str]]:
    """Argv and env for one genos-lighting capture.

    Each control is a flag only when this binary's help lists it. Scene,
    checkpoint, menu, and the MCP port fall back to the env mirror help
    documents. A binary from before `--mcp-port` still reads `GENOS_MCP_PORT`
    in `listen_port` even when help does not name it; passing the flag is what
    exits with `unknown argument`. `--ui` stays off the command: help says that
    flag keeps overlays in the picture. The capture command sends `ui: false`.
    """
    require_display(display)
    text = binary_help(binary)
    out = absolute_path(out)
    argv = [binary]
    env: dict[str, str] = {}
    if "--headless" in help_flags(text):
        argv.append("--headless")
    _flag_or_env(text, argv, env, "--display", display, None)
    if not _flag_or_env(text, argv, env, "--scene", scene, "GENOS_SCENE"):
        raise Broken("binary help has no scene argument")
    if not _flag_or_env(text, argv, env, "--checkpoint", checkpoint, "GENOS_CHECKPOINT"):
        raise Broken("binary help has no checkpoint argument")
    if not _flag_or_env(text, argv, env, "--mcp-port", str(port), "GENOS_MCP_PORT"):
        env["GENOS_MCP_PORT"] = str(port)
    _flag_or_env(text, argv, env, "--out", str(out), None)
    _flag_or_env(text, argv, env, "--size", size, None)
    _flag_or_env(text, argv, env, "--menu", "hidden", "GENOS_MENU")
    return argv, env


def lighting_argv(
    binary: str,
    display: str,
    scene: str,
    checkpoint: str,
    port: int,
    out: Path,
    size: str = "320x180",
) -> list[str]:
    argv, _env = lighting_launch(binary, display, scene, checkpoint, port, out, size)
    return argv


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
    require_display(display)
    if accepts_display_flag(binary):
        return [argv[0], "--display", display, *argv[1:]]
    return argv


def child_env(
    parent: dict[str, str],
    display: str,
    extra: dict[str, str] | None = None,
) -> dict[str, str]:
    """The child sees only the requested Wayland display, not the session compositor."""
    require_display(display)
    env = dict(parent)
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("DISPLAY", None)
    env["WAYLAND_DISPLAY"] = display
    if extra:
        env.update(extra)
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
