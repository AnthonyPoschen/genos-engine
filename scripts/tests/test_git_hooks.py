#!/usr/bin/env python3
"""The pre-push hook runs scripts/lint.sh and does not replace other hooks."""

from __future__ import annotations

import hashlib
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INSTALLER = ROOT / "scripts" / "install-git-hooks.sh"
HOOK = ROOT / "scripts" / "hooks" / "pre-push"
LINT = ROOT / "scripts" / "lint.sh"
MARKER = "# genos-engine: pre-push lint"

DISPATCHER = """#!/bin/sh
set -e
orig=$(cat "$(dirname "$0")/.cursor-original-hooks-path")
name=$(basename "$0")
if [ -x "$orig/$name" ]; then
  "$orig/$name" "$@"
fi
"""


class HookScriptTest(unittest.TestCase):
    def test_committed_hook_invokes_this_repo_lint_sh(self) -> None:
        text = HOOK.read_text(encoding="utf-8")
        self.assertIn("scripts/lint.sh", text)
        self.assertIn("CI remains the authority", text)
        self.assertTrue(LINT.is_file())
        self.assertIn("cargo deny check", LINT.read_text(encoding="utf-8"))
        root = (HOOK.parent / ".." / "..").resolve()
        self.assertEqual(root, ROOT)
        self.assertEqual((root / "scripts" / "lint.sh").resolve(), LINT.resolve())

    def test_real_lint_sh_failure_propagates(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            hook = repo / "scripts" / "hooks" / "pre-push"
            hook.parent.mkdir(parents=True)
            shutil.copy(HOOK, hook)
            lint = repo / "scripts" / "lint.sh"
            shutil.copy(LINT, lint)
            hook.chmod(0o755)
            lint.chmod(0o755)
            self.assertEqual(hook.read_bytes(), HOOK.read_bytes())
            self.assertEqual(lint.read_bytes(), LINT.read_bytes())
            direct = subprocess.run([str(lint)], check=False, text=True, capture_output=True)
            via = subprocess.run([str(hook)], check=False, text=True, capture_output=True)
        self.assertNotEqual(direct.returncode, 0)
        self.assertEqual(via.returncode, direct.returncode)
        self.assertIn("lint-env-mirrors.py", via.stderr + direct.stderr)

    def test_missing_lint_sh_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            hook = repo / "scripts" / "hooks" / "pre-push"
            hook.parent.mkdir(parents=True)
            shutil.copy(HOOK, hook)
            hook.chmod(0o755)
            proc = subprocess.run([str(hook)], check=False, text=True, capture_output=True)
        self.assertEqual(proc.returncode, 1)
        self.assertIn("missing", proc.stderr)


class InstallerTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self._tmp.name)
        self.git("init", "-b", "main")
        self.git("config", "user.email", "test@example.com")
        self.git("config", "user.name", "Test")
        self.git("config", "commit.gpgsign", "false")
        scripts = self.repo / "scripts"
        (scripts / "hooks").mkdir(parents=True)
        shutil.copy(INSTALLER, scripts / "install-git-hooks.sh")
        shutil.copy(HOOK, scripts / "hooks" / "pre-push")
        (scripts / "install-git-hooks.sh").chmod(0o755)
        (scripts / "hooks" / "pre-push").chmod(0o755)
        self.write_lint("#!/bin/sh\necho LINT\nexit 0\n")

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def git(self, *args: str) -> str:
        proc = subprocess.run(
            ["git", *args],
            cwd=self.repo,
            check=True,
            text=True,
            capture_output=True,
        )
        return proc.stdout.strip()

    def write_lint(self, text: str) -> None:
        path = self.repo / "scripts" / "lint.sh"
        path.write_text(text, encoding="utf-8")
        path.chmod(0o755)

    def install(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.repo / "scripts" / "install-git-hooks.sh")],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
        )

    def test_fresh_install_is_idempotent_and_leaves_samples(self) -> None:
        sample = self.repo / ".git" / "hooks" / "pre-push.sample"
        before = sample.read_bytes() if sample.is_file() else None
        other = self.repo / ".git" / "hooks" / "commit-msg.sample"
        other_before = other.read_bytes() if other.is_file() else None
        first = self.install()
        self.assertEqual(first.returncode, 0, first.stderr)
        hook = self.repo / ".git" / "hooks" / "pre-push"
        text = hook.read_text(encoding="utf-8")
        self.assertEqual(text.count(MARKER), 1)
        self.assertIn("show-toplevel", text)
        self.assertNotIn(str(self.repo), text)
        self.assertTrue(hook.stat().st_mode & stat.S_IXUSR)
        second = self.install()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(hook.read_text(encoding="utf-8"), text)
        if before is not None:
            self.assertEqual(sample.read_bytes(), before)
        if other_before is not None:
            self.assertEqual(other.read_bytes(), other_before)
        proc = subprocess.run(
            [str(hook)],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("LINT", proc.stdout)

    def test_existing_hook_keeps_its_commands_and_lint_failure_stops_it(self) -> None:
        hook = self.repo / ".git" / "hooks" / "pre-push"
        hook.write_text("#!/bin/sh\necho USER_HOOK\nexit 0\n", encoding="utf-8")
        hook.chmod(0o755)
        commit_msg = self.repo / ".git" / "hooks" / "commit-msg"
        commit_msg.write_text("#!/bin/sh\necho COMMIT\n", encoding="utf-8")
        commit_before = commit_msg.read_bytes()
        installed = self.install()
        self.assertEqual(installed.returncode, 0, installed.stderr)
        text = hook.read_text(encoding="utf-8")
        self.assertEqual(text.count(MARKER), 1)
        self.assertEqual(text.count("USER_HOOK"), 1)
        self.assertLess(text.index(MARKER), text.index("USER_HOOK"))
        self.assertTrue(text.startswith("#!/bin/sh\n"))
        again = self.install()
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertEqual(hook.read_text(encoding="utf-8"), text)
        self.assertEqual(commit_msg.read_bytes(), commit_before)
        ok = subprocess.run(
            [str(hook)],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
        )
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.assertEqual(ok.stdout.splitlines(), ["LINT", "USER_HOOK"])
        self.write_lint("#!/bin/sh\necho LINT\nexit 4\n")
        failed = subprocess.run(
            [str(hook)],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
        )
        self.assertEqual(failed.returncode, 4)
        self.assertEqual(failed.stdout.splitlines(), ["LINT"])

    def test_binary_hook_is_not_overwritten(self) -> None:
        hook = self.repo / ".git" / "hooks" / "pre-push"
        original = b"#!/bin/sh\n\0echo leave-me\n"
        hook.write_bytes(original)
        proc = self.install()
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("not a text hook", proc.stderr)
        self.assertEqual(hook.read_bytes(), original)

    def test_hooks_path_receives_the_hook_and_keeps_neighbors(self) -> None:
        custom = self.repo / "custom-hooks"
        custom.mkdir()
        neighbor = custom / "commit-msg"
        neighbor.write_text("#!/bin/sh\necho COMMIT\n", encoding="utf-8")
        before = neighbor.read_bytes()
        self.git("config", "core.hooksPath", "custom-hooks")
        proc = self.install()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertFalse((self.repo / ".git" / "hooks" / "pre-push").exists())
        installed = (custom / "pre-push").read_text(encoding="utf-8")
        self.assertIn(MARKER, installed)
        self.assertEqual(neighbor.read_bytes(), before)
        self.install()
        self.assertEqual((custom / "pre-push").read_text(encoding="utf-8"), installed)

    def test_dispatcher_symlink_is_extended_not_replaced(self) -> None:
        hooks = self.repo / "cursor-hooks"
        hooks.mkdir()
        dispatcher = hooks / ".dispatcher"
        dispatcher.write_text(DISPATCHER, encoding="utf-8")
        dispatcher.chmod(0o755)
        (hooks / "commit-msg").symlink_to(".dispatcher")
        original_hooks = self.repo / ".git" / "hooks"
        (hooks / ".cursor-original-hooks-path").write_text(str(original_hooks) + "\n", encoding="utf-8")
        digest = hashlib.sha256(dispatcher.read_bytes()).hexdigest()
        link = os.readlink(hooks / "commit-msg")
        self.git("config", "core.hooksPath", str(hooks))
        proc = self.install()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(os.readlink(hooks / "pre-push"), ".dispatcher")
        self.assertEqual(os.readlink(hooks / "commit-msg"), link)
        self.assertEqual(hashlib.sha256(dispatcher.read_bytes()).hexdigest(), digest)
        chained = (original_hooks / "pre-push").read_text(encoding="utf-8")
        self.assertIn(MARKER, chained)
        self.assertTrue((hooks / "pre-push").is_symlink())
        self.write_lint("#!/bin/sh\necho LINT\nexit 9\n")
        ran = subprocess.run(
            [str(hooks / "pre-push")],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
        )
        self.assertEqual(ran.returncode, 9)
        self.assertIn("LINT", ran.stdout)
        self.install()
        self.assertEqual(os.readlink(hooks / "pre-push"), ".dispatcher")
        self.assertEqual((original_hooks / "pre-push").read_text(encoding="utf-8"), chained)

    def test_existing_pre_push_symlink_is_not_replaced(self) -> None:
        hooks = self.repo / "cursor-hooks"
        hooks.mkdir()
        dispatcher = hooks / ".dispatcher"
        dispatcher.write_text(DISPATCHER, encoding="utf-8")
        dispatcher.chmod(0o755)
        (hooks / "pre-push").symlink_to(".dispatcher")
        original_hooks = self.repo / ".git" / "hooks"
        (hooks / ".cursor-original-hooks-path").write_text(str(original_hooks), encoding="utf-8")
        before = hashlib.sha256(dispatcher.read_bytes()).hexdigest()
        self.git("config", "core.hooksPath", str(hooks))
        proc = self.install()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertTrue((hooks / "pre-push").is_symlink())
        self.assertEqual(os.readlink(hooks / "pre-push"), ".dispatcher")
        self.assertEqual(hashlib.sha256(dispatcher.read_bytes()).hexdigest(), before)
        self.assertIn(MARKER, (original_hooks / "pre-push").read_text(encoding="utf-8"))

    def test_hooks_path_scripts_hooks_does_not_wrap_its_own_hook(self) -> None:
        hook = self.repo / "scripts" / "hooks" / "pre-push"
        before = hook.read_bytes()
        self.git("config", "core.hooksPath", "scripts/hooks")
        proc = self.install()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("not wrapping", proc.stderr)
        self.assertEqual(hook.read_bytes(), before)
        self.assertNotIn(MARKER, hook.read_text(encoding="utf-8"))
        self.write_lint("#!/bin/sh\necho LINT\nexit 0\n")
        try:
            ran = subprocess.run(
                [str(hook)],
                cwd=self.repo,
                check=False,
                text=True,
                capture_output=True,
                timeout=2,
            )
        except subprocess.TimeoutExpired as exc:
            self.fail(f"pre-push called itself: {exc}")
        self.assertEqual(ran.returncode, 0, ran.stderr)
        self.assertEqual(ran.stdout.splitlines(), ["LINT"])
        self.assertEqual(hook.read_bytes(), before)

    def test_binary_original_hook_does_not_leave_a_dispatcher_symlink(self) -> None:
        hooks = self.repo / "cursor-hooks"
        hooks.mkdir()
        dispatcher = hooks / ".dispatcher"
        dispatcher.write_text(DISPATCHER, encoding="utf-8")
        dispatcher.chmod(0o755)
        (hooks / "commit-msg").symlink_to(".dispatcher")
        original_hooks = self.repo / ".git" / "hooks"
        binary = b"#!/bin/sh\n\0echo original\n"
        (original_hooks / "pre-push").write_bytes(binary)
        (hooks / ".cursor-original-hooks-path").write_text(str(original_hooks) + "\n", encoding="utf-8")
        before_names = sorted(os.listdir(hooks))
        dispatcher_bytes = dispatcher.read_bytes()
        link = os.readlink(hooks / "commit-msg")
        self.git("config", "core.hooksPath", str(hooks))
        proc = self.install()
        self.assertNotEqual(proc.returncode, 0, proc.stdout)
        self.assertIn("not a text hook", proc.stderr)
        self.assertEqual(sorted(os.listdir(hooks)), before_names)
        self.assertFalse((hooks / "pre-push").exists())
        self.assertEqual((original_hooks / "pre-push").read_bytes(), binary)
        self.assertEqual(dispatcher.read_bytes(), dispatcher_bytes)
        self.assertEqual(os.readlink(hooks / "commit-msg"), link)


if __name__ == "__main__":
    unittest.main()
