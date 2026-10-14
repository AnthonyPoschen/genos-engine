"""One verdict line, one evidence directory, exit codes 0/1/2/3."""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
from dataclasses import dataclass
from pathlib import Path

PASS = "PASS"
FAIL = "FAIL"
UNSURE = "UNSURE"
CODE = {PASS: 0, FAIL: 1, UNSURE: 2}
BROKEN = 3


class Broken(Exception):
    """The check itself failed. Exit 3. Not a product fail."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


@dataclass
class Verdict:
    status: str
    reason: str
    evidence: Path
    code: int | None = None

    def exit_code(self) -> int:
        if self.code is not None:
            return self.code
        return CODE[self.status]


def repo_root() -> Path:
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (parent / "Cargo.toml").is_file() and (parent / "AGENTS.md").is_file():
            return parent
    raise Broken("repository root not found")


def safe_name(text: str) -> str:
    cleaned = text.strip()
    if (
        not cleaned
        or "/" in cleaned
        or "\\" in cleaned
        or ".." in cleaned
        or any(ord(ch) < 32 for ch in cleaned)
    ):
        return "unknown"
    return cleaned


def evidence_dir(kind: str, sha: str, out: str | None) -> Path:
    if out:
        return Path(out)
    return Path("genos-debug") / "verdict" / safe_name(kind) / safe_name(sha)


def one_line(text: str) -> str:
    return " ".join(text.split()) or "no reason"


def emit(verdict: Verdict) -> None:
    reason = one_line(verdict.reason)
    line = f"{verdict.status} {reason} evidence={verdict.evidence}"
    try:
        verdict.evidence.mkdir(parents=True, exist_ok=True)
        (verdict.evidence / "verdict.txt").write_text(line + "\n", encoding="utf-8")
    except OSError as err:
        print(f"{UNSURE} check-broke {err} evidence={verdict.evidence}", flush=True)
        raise SystemExit(BROKEN) from err
    print(line, flush=True)
    raise SystemExit(verdict.exit_code())


def broken(reason: str, evidence: Path) -> None:
    emit(Verdict(UNSURE, f"check-broke {reason}", evidence, BROKEN))


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--out", help="evidence directory")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate arguments, write them, and do not run the check",
    )
    parser.add_argument(
        "--display",
        help="Wayland display name or compositor socket. Required before a window opens.",
    )
    parser.add_argument("--sha", help="commit sha (default: git rev-parse HEAD)")
    parser.add_argument(
        "--key",
        action="append",
        default=[],
        help="extra cache-key text, repeatable",
    )
    parser.add_argument(
        "--input",
        action="append",
        default=[],
        help="file hashed into the cache key, repeatable",
    )
    parser.add_argument("--cache-dir", help="verdict cache directory")


def mirror(value: str | None, env_name: str) -> str | None:
    """A flag wins. The environment variable is read only when the flag was omitted."""
    if value is not None and str(value).strip() != "":
        return str(value).strip()
    found = os.environ.get(env_name)
    if found is None:
        return None
    found = found.strip()
    return found or None


def apply_common_mirrors(ns: argparse.Namespace) -> None:
    ns.out = mirror(ns.out, "GENOS_VERDICT_OUT")
    ns.display = mirror(ns.display, "GENOS_VERDICT_DISPLAY")
    ns.sha = mirror(ns.sha, "GENOS_VERDICT_SHA")
    ns.cache_dir = mirror(ns.cache_dir, "GENOS_VERDICT_CACHE_DIR")


def input_hash(sha: str, keys: list[str], inputs: list[str]) -> str:
    digest = hashlib.sha256()
    digest.update(sha.encode())
    for key in keys:
        digest.update(b"\0key\0")
        digest.update(key.encode())
    for raw in inputs:
        path = Path(raw)
        if not path.is_file():
            raise Broken(f"input missing {raw}")
        digest.update(b"\0file\0")
        digest.update(str(path).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def write_args(evidence: Path, argv: list[str]) -> None:
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence / "args.txt").write_text("\n".join(argv) + "\n", encoding="utf-8")


def dry_verdict(evidence: Path, argv: list[str]) -> Verdict:
    write_args(evidence, argv)
    (evidence / "probation.txt").write_text(
        "probation\nA miss becomes a test in scripts/verdict/tests/test_misses.py\n",
        encoding="utf-8",
    )
    return Verdict(PASS, "dry-run", evidence)


def run_main(evidence_holder: list[Path], body) -> None:
    """Print one line. `evidence_holder` is updated by `body` when the directory is known."""
    if not evidence_holder:
        evidence_holder.append(Path("genos-debug/verdict/unknown/unknown"))
    try:
        verdict = body()
    except Broken as err:
        broken(err.reason, evidence_holder[0])
    except SystemExit:
        raise
    except Exception as err:  # noqa: BLE001 — the check broke; do not guess a product verdict
        broken(f"{type(err).__name__}: {err}", evidence_holder[0])
    else:
        emit(verdict)


def parser_error(parser: argparse.ArgumentParser, message: str) -> None:
    """Argument errors are a broken check, not argparse's exit 2."""
    evidence = Path("genos-debug/verdict/unknown/unknown")
    broken(message, evidence)
    raise AssertionError("broken exits")


def require_display(display: str | None) -> str:
    if not display or not display.strip():
        raise Broken("missing --display")
    name = display.strip()
    if name in {"HDMI-A-1", "DP-1"}:
        raise Broken("display name is a connector, pass a Wayland display")
    if "/" in name or "\\" in name or name.startswith("."):
        raise Broken("display name must be a Wayland display, not a path")
    return name


def silence_argv(argv: list[str] | None) -> list[str]:
    return list(sys.argv[1:] if argv is None else argv)
