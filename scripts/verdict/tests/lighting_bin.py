"""Build genos-lighting at a commit and cache the binary under target/."""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

from lib.contract import repo_root


def lighting_binary(rev: str) -> Path:
    """Debug `genos-lighting` for `rev`. A later call reuses target/verdict-bins."""
    repo = repo_root()
    full = subprocess.check_output(
        ["git", "rev-parse", rev],
        cwd=repo,
        text=True,
    ).strip()
    dest = repo / "target" / "verdict-bins" / full / "genos-lighting"
    if dest.is_file():
        return dest
    work = Path(os.environ.get("TMPDIR", "/tmp")) / f"verdict-lighting-{full[:12]}"
    try:
        if work.exists():
            subprocess.run(
                ["git", "worktree", "remove", "--force", str(work)],
                cwd=repo,
                check=False,
                capture_output=True,
                text=True,
            )
        added = subprocess.run(
            ["git", "worktree", "add", "--detach", str(work), full],
            cwd=repo,
            check=False,
            text=True,
            capture_output=True,
        )
        if added.returncode != 0:
            raise RuntimeError((added.stderr or added.stdout).strip() or "worktree add failed")
        built = subprocess.run(
            ["cargo", "+stable", "build", "--locked", "-p", "genos-lighting"],
            cwd=work,
            check=False,
            text=True,
            capture_output=True,
        )
        if built.returncode != 0:
            raise RuntimeError((built.stderr or built.stdout)[-4000:] or "cargo build failed")
        binary = work / "target" / "debug" / "genos-lighting"
        if not binary.is_file():
            raise RuntimeError(f"missing {binary}")
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, dest)
        dest.chmod(0o755)
        return dest
    finally:
        subprocess.run(
            ["git", "worktree", "remove", "--force", str(work)],
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
