"""Run every verdict command in docs/VERIFY.md as a real subprocess.

The caller cwd is not the checkout. A stub genos-lighting answers MCP and
writes the capture the script asked for. Cargo is a copy of that stub into
the worktree, so --live still walks the real build and noise-floor path.
"""

from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.contract import repo_root
from lib.images import solid, write_png
from lib.launch import binary_help, help_flags
from lighting_bin import lighting_binary


STUB_LIGHTING = r'''#!/usr/bin/env python3
"""Stub genos-lighting. --help documents --display. Otherwise it answers MCP."""

from __future__ import annotations

import json
import os
import struct
import sys
import threading
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def tiny_png() -> bytes:
    width, height = 4, 2
    raw = bytearray()
    for _y in range(height):
        raw.append(0)
        raw.extend(bytes([40, 80, 120]) * width)

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    compressed = zlib.compress(bytes(raw))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", compressed) + chunk(b"IEND", b"")


PNG = tiny_png()


def log(event: dict) -> None:
    path = os.environ.get("GENOS_STUB_LOG")
    if not path:
        return
    line = (json.dumps(event, sort_keys=True) + "\n").encode()
    fd = os.open(path, os.O_CREAT | os.O_APPEND | os.O_WRONLY, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


def flag(name: str) -> str | None:
    argv = sys.argv[1:]
    for index, item in enumerate(argv):
        if item == name and index + 1 < len(argv):
            return argv[index + 1]
        if item.startswith(name + "="):
            return item.split("=", 1)[1]
    return None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        return

    def _json(self, payload: dict) -> None:
        raw = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self) -> None:  # noqa: N802
        if self.path.rstrip("/") == "/cmd":
            self._json({"ok": True})
            return
        self.send_error(404)

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0") or "0")
        raw = self.rfile.read(length) if length else b"{}"
        try:
            body = json.loads(raw.decode() or "{}")
        except json.JSONDecodeError:
            body = {}
        name = self.path.rstrip("/").rsplit("/", 1)[-1]
        if name == "wait_settled":
            self._json({"settled": True, "frames": 12})
            return
        if name == "timing":
            self._json({"frame_ms": 16.0})
            return
        if name == "capture":
            path = str(body.get("path") or "")
            log({"event": "capture", "path": path, "ui": body.get("ui")})
            if path:
                dest = Path(path)
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(PNG)
            self._json({"ok": True})
            return
        if name == "quit":
            self._json({"ok": True})
            threading.Thread(target=self.server.shutdown, daemon=True).start()
            return
        self._json({"ok": True})


def documented_flags(text: str) -> set[str]:
    found: set[str] = set()
    for token in text.split():
        if not token.startswith("--") or token == "--":
            continue
        name = token.split("=", 1)[0].rstrip(",.;:")
        if len(name) > 2 and all(ch.isalnum() or ch == "-" for ch in name[2:]):
            found.add(name)
    return found


def main() -> None:
    help_path = os.environ.get("GENOS_STUB_HELP")
    if not help_path:
        sys.stderr.write("stub genos-lighting: missing GENOS_STUB_HELP\n")
        sys.exit(2)
    text = Path(help_path).read_text(encoding="utf-8")
    allowed = documented_flags(text)
    if "--help" in sys.argv or "-h" in sys.argv:
        sys.stdout.write(text if text.endswith("\n") else text + "\n")
        return
    for item in sys.argv[1:]:
        if not item.startswith("--"):
            continue
        name = item.split("=", 1)[0]
        if name not in allowed:
            sys.stderr.write(f"genos-lighting: unknown argument {name}\n")
            sys.exit(1)
    port = flag("--mcp-port") or os.environ.get("GENOS_MCP_PORT")
    if port is None:
        sys.stderr.write("stub genos-lighting: missing --mcp-port\n")
        sys.exit(2)
    log(
        {
            "event": "launch",
            "argv": sys.argv,
            "wayland": os.environ.get("WAYLAND_DISPLAY"),
            "display_set": "DISPLAY" in os.environ,
        }
    )
    server = ThreadingHTTPServer(("127.0.0.1", int(port)), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
'''


STUB_CARGO = r'''#!/usr/bin/env python3
"""Stand-in for `cargo build`. Copies the stub binary into the worktree target."""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path


def main() -> None:
    manifest = None
    argv = sys.argv[1:]
    for index, item in enumerate(argv):
        if item == "--manifest-path" and index + 1 < len(argv):
            manifest = Path(argv[index + 1])
        elif item.startswith("--manifest-path="):
            manifest = Path(item.split("=", 1)[1])
    if manifest is None:
        sys.stderr.write("stub cargo: missing --manifest-path\n")
        sys.exit(1)
    source = os.environ.get("GENOS_STUB_BINARY")
    if not source or not Path(source).is_file():
        sys.stderr.write("stub cargo: missing GENOS_STUB_BINARY\n")
        sys.exit(1)
    dest = manifest.parent / "target" / "release" / "genos-lighting"
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy(source, dest)
    dest.chmod(0o755)
    sys.stdout.write("stub cargo\n")


if __name__ == "__main__":
    main()
'''


DOCUMENTED = (
    "precheck.py",
    "noise-floor.py",
    "ab-verdict.py",
    "checkpoint-catalog.py",
    "pose-diff.py",
    "pr-watch.py",
)

TIMEOUT = {
    "precheck.py": 90,
    "noise-floor.py": 120,
    "ab-verdict.py": 240,
    "checkpoint-catalog.py": 30,
    "pose-diff.py": 30,
    "pr-watch.py": 60,
}


def documented_invocations(text: str) -> list[list[str]]:
    """Shell commands in VERIFY.md that start `python3 scripts/verdict/`."""
    commands: list[list[str]] = []
    for block in re.findall(r"```sh\n(.*?)```", text, re.S):
        buf = ""
        for line in block.splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            buf += stripped
            if buf.endswith("\\"):
                buf = buf[:-1] + " "
                continue
            command = buf.strip()
            buf = ""
            if command.startswith("python3 scripts/verdict/"):
                commands.append(shlex.split(command))
    return commands


def write_executable(path: Path, source: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source, encoding="utf-8")
    path.chmod(0o755)


def _run(
    argv: list[str],
    cwd: Path,
    env: dict[str, str],
    timeout: int,
    name: str,
) -> subprocess.CompletedProcess[str]:
    proc = subprocess.Popen(
        argv,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        stdout, stderr = proc.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            stdout, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            stdout, stderr = "", ""
        raise AssertionError(f"{name} timed out after {timeout}s\nstdout={stdout}\nstderr={stderr}") from None
    return subprocess.CompletedProcess(argv, proc.returncode or 0, stdout, stderr)


def events(path: Path) -> list[dict]:
    if not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def drop_worktrees(repo: Path, root: Path) -> None:
    listed = subprocess.run(
        ["git", "worktree", "list", "--porcelain"],
        cwd=repo,
        check=False,
        text=True,
        capture_output=True,
    )
    root_resolved = root.resolve()
    for line in listed.stdout.splitlines():
        if not line.startswith("worktree "):
            continue
        path = Path(line.split(" ", 1)[1])
        try:
            path.resolve().relative_to(root_resolved)
        except ValueError:
            continue
        subprocess.run(
            ["git", "worktree", "remove", "--force", str(path)],
            cwd=repo,
            check=False,
            capture_output=True,
            text=True,
        )
    subprocess.run(
        ["git", "worktree", "prune"],
        cwd=repo,
        check=False,
        capture_output=True,
        text=True,
    )


def worktrees_under(repo: Path, root: Path) -> list[str]:
    listed = subprocess.run(
        ["git", "worktree", "list", "--porcelain"],
        cwd=repo,
        check=False,
        text=True,
        capture_output=True,
    )
    root_resolved = root.resolve()
    found: list[str] = []
    for line in listed.stdout.splitlines():
        if not line.startswith("worktree "):
            continue
        path = Path(line.split(" ", 1)[1])
        try:
            path.resolve().relative_to(root_resolved)
        except ValueError:
            continue
        found.append(str(path))
    return found


class DocumentedLiveTests(unittest.TestCase):
    def test_documented_commands_run_live_from_another_cwd(self) -> None:
        repo = repo_root()
        invocations = documented_invocations((repo / "docs" / "VERIFY.md").read_text(encoding="utf-8"))
        names = [Path(tokens[1]).name for tokens in invocations]
        self.assertEqual(names, list(DOCUMENTED))
        with tempfile.TemporaryDirectory() as tmp:
            caller = Path(tmp)
            try:
                self._run_documented(repo, caller, invocations)
            finally:
                drop_worktrees(repo, caller)
            self.assertEqual(worktrees_under(repo, caller), [])

    def _run_documented(self, repo: Path, caller: Path, invocations: list[list[str]]) -> None:
        stub = caller / "bin" / "genos-lighting"
        cargo = caller / "bin" / "cargo"
        write_executable(stub, STUB_LIGHTING)
        write_executable(cargo, STUB_CARGO)
        release = caller / "target" / "release"
        release.mkdir(parents=True)
        installed = release / "genos-lighting"
        shutil.copy(stub, installed)
        installed.chmod(0o755)
        log_path = caller / "stub.jsonl"
        run = caller / "run"
        run.mkdir()
        (run / "repeats.csv").write_text(
            "scene,checkpoint,blur_rel,flip_mean\n"
            "stress-still,hall,0.01,0.01\n"
            "stress-still,hall,0.01,0.01\n"
            "stress-still,hall,0.01,0.01\n",
            encoding="utf-8",
        )
        picture = solid(4, 2, (0.2, 0.4, 0.6))
        write_png(caller / "previous.png", picture)
        write_png(caller / "pose.png", picture)
        env = os.environ.copy()
        for name in list(env):
            if name.startswith("GENOS_VERDICT_"):
                env.pop(name)
        env["PATH"] = str(caller / "bin") + os.pathsep + env.get("PATH", "")
        env["GENOS_STUB_BINARY"] = str(stub)
        env["GENOS_STUB_LOG"] = str(log_path)
        env["DISPLAY"] = ":1"
        env["WAYLAND_DISPLAY"] = "wayland-0"
        help_path = caller / "gi-v2-help.txt"
        help_path.write_text(binary_help(str(lighting_binary("55795f6"))), encoding="utf-8")
        env["GENOS_STUB_HELP"] = str(help_path)
        self._allowed = help_flags(help_path.read_text(encoding="utf-8"))
        resolved_cargo = shutil.which("cargo", path=env["PATH"])
        self.assertIsNotNone(resolved_cargo)
        self.assertEqual(Path(resolved_cargo).resolve(), cargo.resolve())
        seen: dict[str, tuple[subprocess.CompletedProcess[str], Path]] = {}
        before = 0
        for tokens in invocations:
            name = Path(tokens[1]).name
            argv = [sys.executable, str(repo / tokens[1]), *tokens[2:]]
            proc = _run(argv, caller, env, TIMEOUT[name], name)
            status, evidence = self._verdict(proc, caller, name)
            seen[name] = (proc, evidence)
            if name == "noise-floor.py":
                self._assert_launches(log_path, before, 3)
                before = len(events(log_path))
            if name == "ab-verdict.py":
                self._assert_launches(log_path, before, 3 + 8 * 2)
        self._assert_precheck(seen["precheck.py"])
        self._assert_noise(seen["noise-floor.py"])
        self._assert_ab(seen["ab-verdict.py"])
        self._assert_checkpoint(seen["checkpoint-catalog.py"], caller)
        self._assert_pose(seen["pose-diff.py"])
        self._assert_watch(seen["pr-watch.py"], caller)
        self.assertFalse((caller / "genos-debug" / "verdict" / "gpu.lock").exists())
        self.assertFalse((caller / "genos-debug" / "reference").exists())

    def _verdict(self, proc: subprocess.CompletedProcess[str], caller: Path, name: str) -> tuple[str, Path]:
        detail = f"{name}\nstdout={proc.stdout}\nstderr={proc.stderr}"
        lines = proc.stdout.splitlines()
        self.assertEqual(len(lines), 1, detail)
        line = lines[0]
        status = line.split(" ", 1)[0]
        self.assertIn(status, {"PASS", "FAIL", "UNSURE", "ERROR"}, detail)
        code = {"PASS": 0, "FAIL": 1, "UNSURE": 2, "ERROR": 3}[status]
        self.assertEqual(proc.returncode, code, detail)
        self.assertNotEqual(status, "ERROR", detail)
        evidence = Path(line.split("evidence=", 1)[1].strip())
        self.assertTrue(evidence.is_absolute(), detail)
        evidence.resolve().relative_to(caller.resolve())
        self.assertTrue((evidence / "verdict.txt").is_file(), detail)
        return status, evidence

    def _assert_launches(self, log_path: Path, start: int, launches: int) -> None:
        fresh = events(log_path)[start:]
        started = [item for item in fresh if item.get("event") == "launch"]
        captures = [item for item in fresh if item.get("event") == "capture"]
        self.assertEqual(len(started), launches, fresh)
        self.assertEqual(len(captures), launches, fresh)
        for item in started:
            argv = item["argv"]
            self.assertEqual(item["wayland"], "wayland-1")
            self.assertFalse(item["display_set"])
            self.assertIn("--headless", argv)
            self.assertNotIn("--display", argv)
            self.assertIn("--mcp-port", argv)
            for token in argv[1:]:
                if token.startswith("--"):
                    self.assertIn(token.split("=", 1)[0], self._allowed, argv)
            out_at = argv.index("--out")
            out = Path(argv[out_at + 1])
            self.assertTrue(out.is_absolute(), argv)
            self.assertNotIn("/shots/", str(out))
        for item in captures:
            path = Path(item["path"])
            self.assertTrue(path.is_absolute(), item)
            self.assertNotIn("/shots/", item["path"])
            self.assertTrue(path.is_file(), item)
            self.assertIs(item["ui"], False)

    def _assert_precheck(self, item: tuple[subprocess.CompletedProcess[str], Path]) -> None:
        proc, evidence = item
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASS ", proc.stdout)
        self.assertTrue((evidence / "merge-tree.log").is_file())
        self.assertTrue((evidence / "gpu.txt").is_file())

    def _assert_noise(self, item: tuple[subprocess.CompletedProcess[str], Path]) -> None:
        proc, evidence = item
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASS spread frame_ms=", proc.stdout)
        spread = json.loads((evidence / "spread.json").read_text(encoding="utf-8"))
        self.assertEqual(spread["frame_ms"]["spread"], 0)
        self.assertEqual(spread["repeats"], 3)
        for index in (1, 2, 3):
            shot = evidence / f"run-{index}" / "pose.png"
            self.assertTrue(shot.is_file(), shot)
            self.assertTrue(shot.is_absolute())

    def _assert_ab(self, item: tuple[subprocess.CompletedProcess[str], Path]) -> None:
        proc, evidence = item
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASS 5 scenes within noise ", proc.stdout)
        source = (evidence / "noise-floor" / "source.txt").read_text(encoding="utf-8")
        self.assertIn("base gi-v2\n", source)
        self.assertIn("scene stress\n", source)
        self.assertIn("checkpoint hall\n", source)
        self.assertIn("repeats 3\n", source)
        self.assertTrue((evidence / "noise-floor" / "spread.json").is_file())
        self.assertIn("stub cargo", (evidence / "build-base.log").read_text(encoding="utf-8"))
        self.assertIn("stub cargo", (evidence / "build-head.log").read_text(encoding="utf-8"))
        scenes = (evidence / "scenes.csv").read_text(encoding="utf-8").strip().splitlines()
        self.assertEqual(scenes[0], "scene,status")
        self.assertEqual(len(scenes), 6)
        self.assertTrue(all(line.endswith(",PASS") for line in scenes[1:]))
        views = (evidence / "views.csv").read_text(encoding="utf-8").strip().splitlines()
        self.assertEqual(len(views), 9)
        self.assertTrue(all(line.split(",")[2] == "PASS" for line in views[1:]))

    def _assert_checkpoint(self, item: tuple[subprocess.CompletedProcess[str], Path], caller: Path) -> None:
        proc, evidence = item
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue(proc.stdout.startswith("PASS "))
        self.assertIn("reference images are not locked", (evidence / "references.txt").read_text(encoding="utf-8"))
        self.assertFalse((evidence / "reference").exists())
        self.assertFalse((caller / "genos-debug" / "reference").exists())

    def _assert_pose(self, item: tuple[subprocess.CompletedProcess[str], Path]) -> None:
        proc, evidence = item
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue(proc.stdout.startswith("PASS "))
        self.assertIn('"ui": false', (evidence / "capture.json").read_text(encoding="utf-8"))
        self.assertTrue((evidence / "diff.png").is_file())
        self.assertTrue((evidence / "numbers.csv").is_file())

    def _assert_watch(self, item: tuple[subprocess.CompletedProcess[str], Path], caller: Path) -> None:
        proc, evidence = item
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASS baseline ", proc.stdout)
        self.assertEqual((evidence / "new-heads.txt").read_text(encoding="utf-8"), "")
        state = caller / "genos-debug" / "verdict" / "pr-watch" / "state.json"
        self.assertTrue(state.is_file(), proc.stdout + proc.stderr)
        saved = json.loads(state.read_text(encoding="utf-8"))
        self.assertIsInstance(saved, dict)


if __name__ == "__main__":
    unittest.main()
