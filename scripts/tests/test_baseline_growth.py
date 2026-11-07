#!/usr/bin/env python3
"""Regression tests for the proposed baseline-growth gate.

The label name and the owner line are a proposal. These tests lock that
contract. They do not create a GitHub label and they do not edit the
repository baselines.

The default suite is hermetic. It does not fetch GitHub. Set
GENOS_BASELINE_GROWTH_LIVE_STACK=1 to add
test_actual_stack_175_base_lacks_the_script_and_still_gates_growth, which
fetches pull request 175. scripts/lint.sh and --self-test do not set that
variable.
"""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "lint-baseline-growth.py"
CI_SCRIPT = ROOT / "scripts" / "lint-baseline-growth-ci.sh"
WORKFLOW = ROOT / ".github" / "workflows" / "baseline-growth.yml"
LOSSY = "lint/lossy-cast-baseline.txt"
SHADER = "lint/shader-magic-baseline.txt"
OWNER = "# owner: Ada Lovelace — the cast is a counted GPU size"
EM = "\u2014"
# Pull request 175. The base commit predates the gate script.
STACK_BASE = "c856b2c95ce59ff77fd4258174522e6c3441bb07"
STACK_HEAD = "12f50a53b1f95f88441b3d4c6a02fd166ca614b3"
STACK_REMOTE = "https://github.com/AnthonyPoschen/genos-engine.git"


def workflow_steps(text: str) -> list[str]:
    marker = "\n    steps:\n"
    if marker not in text:
        raise AssertionError("workflow has no steps")
    return text.split(marker, 1)[1].split("\n      - ")


def git_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(extra or {})
    return env


def load_guard():
    spec = importlib.util.spec_from_file_location("lint_baseline_growth", SCRIPT)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    # dataclasses look up the module in sys.modules while the class body runs.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_linter(name: str):
    path = ROOT / "scripts" / name
    spec = importlib.util.spec_from_file_location(name.replace(".py", "").replace("-", "_"), path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GuardTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.guard = load_guard()
        cls.clippy = load_linter("lint-clippy.py")
        cls.shader = load_linter("lint-shader-constants.py")

    def test_proposed_label_constant(self) -> None:
        self.assertEqual(self.guard.PROPOSED_LABEL, "baseline-growth-approved")

    def test_owner_comments_do_not_change_linter_ceilings(self) -> None:
        for relative, module in (
            (LOSSY, self.clippy),
            (SHADER, self.shader),
        ):
            original = (ROOT / relative).read_text(encoding="utf-8")
            annotated = []
            for line in original.splitlines():
                stripped = line.strip()
                if stripped and not stripped.startswith("#"):
                    annotated.append(OWNER)
                annotated.append(line)
            text = "\n".join(annotated) + "\n"
            self.assertEqual(
                self._loader_counts(module.load_baseline, original),
                self._loader_counts(module.load_baseline, text),
                relative,
            )
            self.assertEqual(
                self.guard.effective_counts(text),
                self._loader_counts(module.load_baseline, original),
            )

    def test_real_baselines_are_unchanged_input(self) -> None:
        for relative in (LOSSY, SHADER):
            text = (ROOT / relative).read_text(encoding="utf-8")
            clippy_counts = self._loader_counts(self.clippy.load_baseline, text)
            shader_counts = self._loader_counts(self.shader.load_baseline, text)
            self.assertEqual(clippy_counts, shader_counts)
            self.assertEqual(self.guard.effective_counts(text), clippy_counts)
            self.assertNotIn("baseline-growth-approved", text)

    def test_trailing_comment_is_not_a_compatible_annotation(self) -> None:
        text = f"crates/example.rs 1 # owner: Ada {EM} because\n"
        with self.assertRaises(self.guard.ParseError):
            self.guard.effective_counts(text)

    def test_addition_fails_without_label_and_owner_line(self) -> None:
        base = "crates/a.rs 1\n"
        head = "crates/a.rs 1\ncrates/b.rs 4\n"
        added = self.guard.additions_in_text(base, head, LOSSY)
        self.assertEqual([(item.path, item.old, item.new) for item in added], [("crates/b.rs", 0, 4)])
        outcome = self.guard.judge(added, labels=set(), required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(outcome.code, 1)
        text = "\n".join(outcome.messages)
        self.assertIn("baseline-growth-approved", text)
        self.assertIn("crates/b.rs", text)
        self.assertIn("# owner:", text)
        self.assertIn("proposal", text.lower())

    def test_addition_passes_with_label_and_owner_line(self) -> None:
        base = "crates/a.rs 1\n"
        head = f"crates/a.rs 1\n{OWNER}\ncrates/b.rs 4\n"
        added = self.guard.additions_in_text(base, head, LOSSY)
        outcome = self.guard.judge(
            added,
            labels={self.guard.PROPOSED_LABEL},
            required_label=self.guard.PROPOSED_LABEL,
        )
        self.assertEqual(outcome.code, 0, outcome.messages)

    def test_label_without_owner_line_fails(self) -> None:
        added = self.guard.additions_in_text("crates/a.rs 1\n", "crates/a.rs 2\n", LOSSY)
        outcome = self.guard.judge(added, labels={self.guard.PROPOSED_LABEL}, required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(outcome.code, 1)
        self.assertIn("crates/a.rs", "\n".join(outcome.messages))

    def test_owner_line_without_label_fails(self) -> None:
        head = f"{OWNER}\ncrates/a.rs 2\n"
        added = self.guard.additions_in_text("crates/a.rs 1\n", head, SHADER)
        outcome = self.guard.judge(added, labels=set(), required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(outcome.code, 1)
        self.assertIn("baseline-growth-approved", "\n".join(outcome.messages))

    def test_label_removal_fails_an_addition_that_previously_passed(self) -> None:
        head = f"{OWNER}\ncrates/a.rs 3\n"
        added = self.guard.additions_in_text("crates/a.rs 1\n", head, LOSSY)
        passed = self.guard.judge(added, labels={self.guard.PROPOSED_LABEL}, required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(passed.code, 0)
        removed = self.guard.judge(added, labels=set(), required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(removed.code, 1)
        self.assertIn("baseline-growth-approved", "\n".join(removed.messages))

    def test_removal_passes_without_label_or_rationale(self) -> None:
        base = "crates/a.rs 5\ncrates/b.rs 2\n"
        head = "crates/a.rs 1\n"
        added = self.guard.additions_in_text(base, head, LOSSY)
        self.assertEqual(added, [])
        outcome = self.guard.judge(added, labels=set(), required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(outcome.code, 0)

    def test_no_change_and_reorder_pass(self) -> None:
        base = "crates/a.rs 1\ncrates/b.rs 2\n# a header\ncrates/c.rs 3\n"
        head = "# a header\ncrates/c.rs 3\ncrates/a.rs 1\ncrates/b.rs 2\n"
        self.assertEqual(self.guard.additions_in_text(base, head, LOSSY), [])

    def test_duplicate_lines_use_the_linter_last_wins_count(self) -> None:
        base = "crates/a.rs 2\n"
        same = "crates/a.rs 9\ncrates/a.rs 2\n"
        self.assertEqual(self.guard.additions_in_text(base, same, LOSSY), [])
        grown = "crates/a.rs 2\ncrates/a.rs 9\n"
        added = self.guard.additions_in_text(base, grown, LOSSY)
        self.assertEqual([(item.old, item.new) for item in added], [(2, 9)])
        self.assertIsNone(self.guard.owner_parts(added[0].previous_line))
        annotated = f"crates/a.rs 2\n{OWNER}\ncrates/a.rs 9\n"
        added = self.guard.additions_in_text(base, annotated, LOSSY)
        self.assertIsNotNone(self.guard.owner_parts(added[0].previous_line))

    def test_reordered_duplicates_do_not_count_as_growth(self) -> None:
        base = "crates/b.rs 2\ncrates/a.rs 1\ncrates/a.rs 4\n"
        head = "crates/a.rs 4\ncrates/b.rs 2\n"
        self.assertEqual(self.guard.effective_counts(base), {"crates/b.rs": 2, "crates/a.rs": 4})
        self.assertEqual(self.guard.additions_in_text(base, head, SHADER), [])

    def test_missing_and_empty_rationale(self) -> None:
        self.assertIsNone(self.guard.owner_parts(None))
        self.assertIsNone(self.guard.owner_parts("# still a header"))
        self.assertIsNone(self.guard.owner_parts(f"# owner: {EM} a reason"))
        self.assertIsNone(self.guard.owner_parts("# owner: Ada —"))
        self.assertIsNone(self.guard.owner_parts("# owner: Ada —    "))
        self.assertIsNone(self.guard.owner_parts("# owner:    — a reason"))
        self.assertIsNone(self.guard.owner_parts("# Owner: Ada — a reason"))
        self.assertIsNone(self.guard.owner_parts("# owner: Ada - a reason"))
        self.assertIsNone(self.guard.owner_parts("# owner: Ada – a reason"))
        parsed = self.guard.owner_parts("# owner: Ada Lovelace — fog extinction stays named")
        self.assertEqual(parsed, ("Ada Lovelace", "fog extinction stays named"))

    def test_blank_line_or_shared_comment_is_not_beside_the_entry(self) -> None:
        base = "crates/a.rs 1\n"
        blank = f"{OWNER}\n\ncrates/a.rs 4\n"
        added = self.guard.additions_in_text(base, blank, LOSSY)
        self.assertIsNone(self.guard.owner_parts(added[0].previous_line))
        shared = f"{OWNER}\ncrates/a.rs 4\ncrates/b.rs 2\n"
        added = self.guard.additions_in_text("crates/a.rs 1\n", shared, LOSSY)
        by_path = {item.path: item for item in added}
        self.assertIsNotNone(self.guard.owner_parts(by_path["crates/a.rs"].previous_line))
        self.assertIsNone(self.guard.owner_parts(by_path["crates/b.rs"].previous_line))

    def test_path_replacement_is_an_addition(self) -> None:
        base = "crates/old.rs 4\n"
        head = f"crates/old.rs 4\n{OWNER}\ncrates/new.rs 4\n"
        added = self.guard.additions_in_text(base, head, LOSSY)
        self.assertEqual([item.path for item in added], ["crates/new.rs"])
        replaced = f"{OWNER}\ncrates/new.rs 4\n"
        added = self.guard.additions_in_text(base, replaced, SHADER)
        self.assertEqual([(item.path, item.old, item.new) for item in added], [("crates/new.rs", 0, 4)])

    def test_zero_count_new_path_does_not_raise_the_ceiling(self) -> None:
        added = self.guard.additions_in_text("crates/a.rs 1\n", "crates/a.rs 1\ncrates/b.rs 0\n", LOSSY)
        self.assertEqual(added, [])

    def test_pull_request_body_is_not_the_rationale(self) -> None:
        added = self.guard.additions_in_text("crates/a.rs 1\n", "crates/a.rs 2\n", LOSSY)
        outcome = self.guard.judge(added, labels={self.guard.PROPOSED_LABEL}, required_label=self.guard.PROPOSED_LABEL)
        self.assertEqual(outcome.code, 1)
        self.assertNotIn("body", self.guard.judge.__code__.co_varnames)

    def test_custom_label_argument_is_the_one_that_counts(self) -> None:
        head = f"{OWNER}\ncrates/a.rs 2\n"
        added = self.guard.additions_in_text("crates/a.rs 1\n", head, LOSSY)
        wrong = self.guard.judge(added, labels={self.guard.PROPOSED_LABEL}, required_label="ceiling-ok")
        self.assertEqual(wrong.code, 1)
        self.assertIn("ceiling-ok", "\n".join(wrong.messages))
        right = self.guard.judge(added, labels={"ceiling-ok"}, required_label="ceiling-ok")
        self.assertEqual(right.code, 0)

    def test_help_documents_the_proposal_and_every_argument(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(SCRIPT), "--help"],
            check=False,
            text=True,
            capture_output=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        help_text = proc.stdout
        for flag in ("--label", "--base-sha", "--head-sha", "--pull", "--repo", "--labels", "--api-base", "--self-test"):
            self.assertIn(flag, help_text)
        self.assertIn("GENOS_BASELINE_GROWTH_LABEL", help_text)
        self.assertIn("baseline-growth-approved", help_text)
        self.assertIn("proposal", help_text.lower())

    def _loader_counts(self, loader, text: str) -> dict[str, int]:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "baseline.txt"
            path.write_text(text, encoding="utf-8")
            return loader(path)


class GitGuardTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.guard = load_guard()

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self._tmp.name)
        self.git("init", "-b", "main")
        self.git("config", "user.email", "test@example.com")
        self.git("config", "user.name", "Test")
        self.git("config", "commit.gpgsign", "false")

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

    def commit(self, message: str, files: dict[str, str]) -> str:
        for name, text in files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
            self.git("add", "--", name)
        self.git("commit", "-m", message)
        return self.git("rev-parse", "HEAD")

    def test_addition_against_the_pull_request_base_sha(self) -> None:
        base = self.commit("base", {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        head = self.commit(
            "grow",
            {SHADER: f"crates/render/shaders/a.comp 1\n{OWNER}\ncrates/render/shaders/b.comp 2\n"},
        )
        failed = self.guard.evaluate(
            self.repo,
            base_sha=base,
            head_sha=head,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(failed.code, 1, failed.messages)
        self.assertIn("crates/render/shaders/b.comp", "\n".join(failed.messages))
        passed = self.guard.evaluate(
            self.repo,
            base_sha=base,
            head_sha=head,
            labels={self.guard.PROPOSED_LABEL},
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(passed.code, 0, passed.messages)

    def test_reduction_and_rename_follow_the_merge_base_not_a_branch_tip(self) -> None:
        self.commit("m1", {LOSSY: "crates/old.rs 5\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        self.git("checkout", "-b", "feature")
        feature = self.commit("shrink and rename", {LOSSY: f"{OWNER}\ncrates/new.rs 4\n"})
        self.git("checkout", "main")
        main = self.commit("main grew the old name", {LOSSY: "crates/old.rs 9\n"})
        self.assertNotEqual(self.git("diff", main, feature), "")
        failed = self.guard.evaluate(
            self.repo,
            base_sha=main,
            head_sha=feature,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(failed.code, 1, failed.messages)
        self.assertIn("crates/new.rs", "\n".join(failed.messages))
        self.assertNotIn("crates/old.rs", "\n".join(failed.messages))

    def test_same_growth_on_the_base_tip_still_counts_on_the_pull_request(self) -> None:
        self.commit("m1", {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        self.git("checkout", "-b", "feature")
        feature = self.commit("feature grew", {LOSSY: "crates/a.rs 2\n"})
        self.git("checkout", "main")
        main = self.commit("main grew the same way", {LOSSY: "crates/a.rs 2\n"})
        diff_two_dot = subprocess.run(
            ["git", "diff", main, feature, "--", LOSSY],
            cwd=self.repo,
            check=True,
            text=True,
            capture_output=True,
        )
        self.assertEqual(diff_two_dot.stdout, "")
        outcome = self.guard.evaluate(
            self.repo,
            base_sha=main,
            head_sha=feature,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(outcome.code, 1, outcome.messages)
        self.assertIn("crates/a.rs", "\n".join(outcome.messages))

    def test_file_rename_keeps_unchanged_entries_and_flags_new_ones(self) -> None:
        base = self.commit(
            "base",
            {LOSSY: "crates/a.rs 2\ncrates/b.rs 3\n", SHADER: "crates/render/shaders/a.comp 1\n"},
        )
        self.git("mv", LOSSY, "lint/casts-baseline.txt")
        renamed = self.repo / "lint" / "casts-baseline.txt"
        renamed.write_text(
            f"crates/a.rs 2\ncrates/b.rs 3\n{OWNER}\ncrates/c.rs 1\n",
            encoding="utf-8",
        )
        self.git("add", "-A")
        self.git("commit", "-m", "rename")
        head = self.git("rev-parse", "HEAD")
        outcome = self.guard.evaluate(
            self.repo,
            base_sha=base,
            head_sha=head,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(outcome.code, 1, outcome.messages)
        message = "\n".join(outcome.messages)
        self.assertIn("crates/c.rs", message)
        self.assertNotIn("crates/a.rs", message)
        self.assertNotIn("crates/b.rs", message)

    def test_base_tip_that_lowers_a_count_does_not_charge_the_pull_request(self) -> None:
        self.commit("m1", {LOSSY: "crates/a.rs 5\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        self.git("checkout", "-b", "feature")
        feature = self.commit("docs only", {"notes.txt": "no baseline edit\n"})
        self.git("checkout", "main")
        main = self.commit("lower on main", {LOSSY: "crates/a.rs 1\n"})
        outcome = self.guard.evaluate(
            self.repo,
            base_sha=main,
            head_sha=feature,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(outcome.code, 0, outcome.messages)
        self.assertIn("no added entries", "\n".join(outcome.messages))

    def test_dissimilar_replacement_file_is_still_growth(self) -> None:
        base = self.commit(
            "base",
            {LOSSY: "crates/a.rs 2\n", SHADER: "crates/render/shaders/a.comp 1\n"},
        )
        (self.repo / LOSSY).unlink()
        replacement = "lint/totally-different-baseline.txt"
        (self.repo / replacement).write_text(
            "\n".join(f"zzz/unrelated_{i}.rs {i}" for i in range(3, 12)) + "\n",
            encoding="utf-8",
        )
        self.git("add", "-A")
        self.git("commit", "-m", "replace")
        head = self.git("rev-parse", "HEAD")
        outcome = self.guard.evaluate(
            self.repo,
            base_sha=base,
            head_sha=head,
            labels={self.guard.PROPOSED_LABEL},
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(outcome.code, 1, outcome.messages)
        self.assertIn("zzz/unrelated_3.rs", "\n".join(outcome.messages))

    def test_unrelated_history_and_bad_sha_fail_closed(self) -> None:
        base = self.commit("base", {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        self.git("checkout", "--orphan", "other")
        self.git("rm", "-rf", ".")
        head = self.commit("orphan", {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        unrelated = self.guard.evaluate(
            self.repo,
            base_sha=base,
            head_sha=head,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(unrelated.code, 2, unrelated.messages)
        self.assertIn("Fail closed", "\n".join(unrelated.messages))
        bad = self.guard.evaluate(
            self.repo,
            base_sha="--version",
            head_sha=head,
            labels=set(),
            required_label=self.guard.PROPOSED_LABEL,
            fetch_labels=self._boom,
        )
        self.assertEqual(bad.code, 2)
        self.assertIn("Fail closed", "\n".join(bad.messages))

    def test_cli_metadata_failure_and_label_argument_precedence(self) -> None:
        base = self.commit("base", {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        head = self.commit("grow", {LOSSY: f"{OWNER}\ncrates/a.rs 2\n"})
        missing = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
            ],
            env={"GITHUB_TOKEN": "", "GH_TOKEN": ""},
        )
        self.assertEqual(missing.returncode, 2, missing.stderr)
        self.assertIn("Fail closed", missing.stderr)
        self.assertNotIn(self.guard.PROPOSED_LABEL, missing.stdout)

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802
                self.send_response(500)
                self.end_headers()
                self.wfile.write(b'{"token":"should-not-leak"}')

            def log_message(self, fmt: str, *args) -> None:
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.shutdown)
        self.addCleanup(server.server_close)
        port = server.server_address[1]
        http = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
                "--api-base",
                f"http://127.0.0.1:{port}",
            ],
            env={"GITHUB_TOKEN": "secret-token", "GH_TOKEN": ""},
        )
        self.assertEqual(http.returncode, 2, http.stderr)
        self.assertIn("HTTP 500", http.stderr)
        self.assertIn("Fail closed", http.stderr)
        self.assertNotIn("secret-token", http.stderr)
        self.assertNotIn("should-not-leak", http.stderr)

        closed = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
                "--api-base",
                "http://127.0.0.1:1",
            ],
            env={"GITHUB_TOKEN": "secret-token"},
        )
        self.assertEqual(closed.returncode, 2, closed.stderr)
        self.assertIn("Fail closed", closed.stderr)
        self.assertNotIn("secret-token", closed.stderr)

        bad_json = self._serve({"not-json": True}, status=200, raw=b"not-json")
        malformed = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
                "--api-base",
                bad_json,
            ],
            env={"GITHUB_TOKEN": "secret-token"},
        )
        self.assertEqual(malformed.returncode, 2, malformed.stderr)
        self.assertIn("Fail closed", malformed.stderr)

        no_labels_key = self._serve({"body": f"# owner: Ada {EM} because"}, status=200)
        bare = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
                "--api-base",
                no_labels_key,
            ],
            env={"GITHUB_TOKEN": "secret-token"},
        )
        self.assertEqual(bare.returncode, 2, bare.stderr)

        labeled = self._serve(
            {"body": "why in the body does not count", "labels": [{"name": self.guard.PROPOSED_LABEL}]},
            status=200,
        )
        ok = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
                "--api-base",
                labeled,
            ],
            env={"GITHUB_TOKEN": "secret-token", "GENOS_BASELINE_GROWTH_LABEL": "someone-else"},
        )
        self.assertEqual(ok.returncode, 1, ok.stderr + ok.stdout)
        self.assertIn("someone-else", ok.stderr)

        explicit = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--labels",
                self.guard.PROPOSED_LABEL,
                "--label",
                self.guard.PROPOSED_LABEL,
            ],
            env={"GENOS_BASELINE_GROWTH_LABEL": "someone-else", "GITHUB_TOKEN": ""},
        )
        self.assertEqual(explicit.returncode, 0, explicit.stderr)
        self.assertIn("proposal contract satisfied", explicit.stdout)

        removed = self._run(
            ["--base-sha", base, "--head-sha", head, "--labels", ""],
            env={"GITHUB_TOKEN": ""},
        )
        self.assertEqual(removed.returncode, 1, removed.stderr)
        self.assertIn("baseline-growth-approved", removed.stderr)

    def test_reduction_does_not_call_the_api(self) -> None:
        base = self.commit("base", {LOSSY: "crates/a.rs 4\n", SHADER: "crates/render/shaders/a.comp 1\n"})
        head = self.commit("shrink", {LOSSY: "crates/a.rs 1\n"})
        proc = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                head,
                "--pull",
                "7",
                "--repo",
                "example/genos",
                "--api-base",
                "http://127.0.0.1:1",
            ],
            env={"GITHUB_TOKEN": ""},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("no added entries", proc.stdout)

    def test_workflow_is_read_only_and_does_not_run_the_head(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("pull_request_target:", text)
        self.assertNotRegex(text, r"(?m)^  pull_request:\s*$")
        self.assertIn("contents: read", text)
        self.assertIn("pull-requests: read", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("pull-requests: write", text)
        for event in ("opened", "synchronize", "reopened", "labeled", "unlabeled", "edited"):
            self.assertIn(event, text)
        self.assertIn("github.event.pull_request.base.sha", text)
        self.assertRegex(text, r"ref:\s*\$\{\{\s*github\.sha\s*\}\}")
        self.assertNotRegex(text, r"ref:\s*\$\{\{\s*github\.event\.pull_request\.base\.sha")
        self.assertNotRegex(text, r"ref:\s*\$\{\{\s*github\.event\.pull_request\.head")
        self.assertNotIn("head.ref", text)
        self.assertIn(self.guard.PROPOSED_LABEL, text)
        self.assertIn("--label", text)
        self.assertNotIn("pull_request_target", (ROOT / ".github" / "workflows" / "lint.yml").read_text(encoding="utf-8"))

    def test_production_workflow_wires_github_token_to_the_gate_step(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        steps = workflow_steps(text)
        gate = [
            step
            for step in steps
            if "lint-baseline-growth.py" in step and "--base-sha" in step and "--pull" in step
        ]
        self.assertEqual(len(gate), 1)
        self.assertRegex(
            gate[0],
            r"(?m)^[ ]+GITHUB_TOKEN:\s*\$\{\{\s*github\.token\s*\}\}\s*$",
        )
        for step in steps:
            if step == gate[0]:
                continue
            self.assertNotIn("github.token", step)
            self.assertNotIn("secrets.GITHUB_TOKEN", step)
            self.assertNotIn("GH_TOKEN", step)
        self.assertNotIn("--labels", text)
        self.assertNotIn("github.event.pull_request.labels", text)
        self.assertNotRegex(text, r"(?i)(contents|pull-requests):\s*write")

    def test_production_workflow_checks_out_the_trusted_default_branch(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        ci = CI_SCRIPT.read_text(encoding="utf-8")
        steps = workflow_steps(text)
        checkout = [step for step in steps if "actions/checkout@" in step]
        self.assertEqual(len(checkout), 1)
        self.assertRegex(checkout[0], r"ref:\s*\$\{\{\s*github\.sha\s*\}\}")
        self.assertNotIn("pull_request.base.sha", checkout[0])
        self.assertNotIn("pull_request.head", checkout[0])
        self.assertNotIn("repository:", checkout[0])
        self.assertIn("persist-credentials: true", checkout[0])
        self.assertIn("fetch-depth: 0", checkout[0])
        fetch = [step for step in steps if "lint-baseline-growth-ci.sh fetch" in step]
        self.assertEqual(len(fetch), 1)
        self.assertIn("TRUSTED_SHA: ${{ github.sha }}", fetch[0])
        self.assertIn("BASE_SHA: ${{ github.event.pull_request.base.sha }}", fetch[0])
        self.assertIn("BASE_REF: ${{ github.event.pull_request.base.ref }}", fetch[0])
        self.assertIn("HEAD_SHA: ${{ github.event.pull_request.head.sha }}", fetch[0])
        self.assertIn("PR_NUMBER: ${{ github.event.pull_request.number }}", fetch[0])
        self.assertNotIn("github.token", fetch[0])
        self.assertNotIn("secrets.", fetch[0])
        self.assertNotIn("GH_TOKEN", fetch[0])
        self.assertNotIn("clone_url", text)
        self.assertNotIn("head.repo", text)
        self.assertNotIn("git checkout", text)
        self.assertNotIn("git switch", text)
        self.assertNotIn("continue-on-error", text)
        self.assertNotRegex(text, r"(?m)^\s+if:")
        gate = next(step for step in steps if "--pull" in step)
        guard_at = gate.index("lint-baseline-growth-ci.sh guard")
        python_at = gate.index("python3 scripts/lint-baseline-growth.py --self-test")
        self.assertLess(guard_at, python_at)
        self.assertNotIn("git checkout", ci)
        self.assertNotIn("git switch", ci)
        self.assertNotIn("clone_url", ci)
        self.assertNotIn("eval ", ci)
        self.assertIn("git fetch --no-tags --no-recurse-submodules origin --", ci)
        self.assertIn("trusted checkout has no gate script", ci)
        self.assertIn('PYTHONDONTWRITEBYTECODE: "1"', gate)

    def test_old_base_without_the_gate_script_still_gates_growth(self) -> None:
        files = {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"}
        base = self.commit("base without the gate", files)
        missing = subprocess.run(
            ["git", "cat-file", "-e", f"{base}:scripts/lint-baseline-growth.py"],
            cwd=self.repo,
            check=False,
            capture_output=True,
            env=git_env(),
        )
        self.assertNotEqual(missing.returncode, 0)
        head = self.commit("same ceilings", {**files, "notes.txt": "no baseline growth\n"})
        also_missing = subprocess.run(
            ["git", "cat-file", "-e", f"{head}:scripts/lint-baseline-growth.py"],
            cwd=self.repo,
            check=False,
            capture_output=True,
            env=git_env(),
        )
        self.assertNotEqual(also_missing.returncode, 0)
        quiet = self._run(["--base-sha", base, "--head-sha", head, "--labels", ""], env={})
        self.assertEqual(quiet.returncode, 0, quiet.stderr)
        self.assertIn("no added entries", quiet.stdout)
        grown = self.commit(
            "grow",
            {LOSSY: "crates/a.rs 1\ncrates/b.rs 4\n", SHADER: files[SHADER]},
        )
        denied = self._run(["--base-sha", base, "--head-sha", grown, "--labels", ""], env={})
        self.assertEqual(denied.returncode, 1, denied.stderr)
        self.assertIn("baseline-growth-approved", denied.stderr)
        self.assertIn("# owner:", denied.stderr)
        self.assertIn("crates/b.rs", denied.stderr)
        owned = self.commit(
            "owned",
            {LOSSY: f"crates/a.rs 1\n{OWNER}\ncrates/b.rs 4\n", SHADER: files[SHADER]},
        )
        allowed = self._run(
            [
                "--base-sha",
                base,
                "--head-sha",
                owned,
                "--labels",
                "baseline-growth-approved",
            ],
            env={},
        )
        self.assertEqual(allowed.returncode, 0, allowed.stderr)
        self.assertIn("owner lines present", allowed.stdout)

    def _live_stack_175(self) -> None:
        fetched = self._fetch_stack()
        missing = subprocess.run(
            ["git", "cat-file", "-e", f"{STACK_BASE}:scripts/lint-baseline-growth.py"],
            cwd=fetched,
            check=False,
            capture_output=True,
            env=git_env(),
        )
        self.assertNotEqual(missing.returncode, 0)
        quiet = self._run_in(
            fetched,
            ["--base-sha", STACK_BASE, "--head-sha", STACK_HEAD, "--labels", ""],
        )
        self.assertEqual(quiet.returncode, 0, quiet.stderr)
        self.assertIn("no added entries", quiet.stdout)

        base_text = subprocess.run(
            ["git", "show", f"{STACK_BASE}:{LOSSY}"],
            cwd=fetched,
            check=True,
            capture_output=True,
            env=git_env(),
        ).stdout.decode("utf-8")
        self.assertNotIn("crates/baseline-growth-stack-probe.rs", base_text)
        if not base_text.endswith("\n"):
            base_text += "\n"
        probe = "crates/baseline-growth-stack-probe.rs 1\n"
        grown = self._commit_baselines(fetched, STACK_BASE, {LOSSY: base_text + probe})
        denied = self._run_in(
            fetched,
            ["--base-sha", STACK_BASE, "--head-sha", grown, "--labels", ""],
        )
        self.assertEqual(denied.returncode, 1, denied.stderr)
        self.assertIn("baseline-growth-approved", denied.stderr)
        self.assertIn("# owner:", denied.stderr)
        self.assertIn("crates/baseline-growth-stack-probe.rs", denied.stderr)
        allowed_head = self._commit_baselines(
            fetched,
            STACK_BASE,
            {LOSSY: base_text + f"{OWNER}\n{probe}"},
        )
        allowed = self._run_in(
            fetched,
            [
                "--base-sha",
                STACK_BASE,
                "--head-sha",
                allowed_head,
                "--labels",
                "baseline-growth-approved",
            ],
        )
        self.assertEqual(allowed.returncode, 0, allowed.stderr)
        self.assertIn("owner lines present", allowed.stdout)

    def test_workflow_fetch_keeps_the_trusted_worktree_and_ignores_a_malicious_head(self) -> None:
        marker = self.repo / "untrusted-ran"
        built = self._trusted_remote(marker)
        work = Path(built["work"])
        self._ci(work, "fetch", self._ci_env(built, pull="9", head=built["same"]))
        self.assertEqual(self._head(work), built["trusted"])
        self.assertFalse(marker.exists())
        self.assertEqual(self._refs(work), built["refs"])
        same = self._gate(work, built, head=built["same"], pull="9", token="", api="http://127.0.0.1:1")
        self.assertEqual(same.returncode, 0, same.stderr)
        self.assertIn("no added entries", same.stdout)
        self.assertFalse(marker.exists())
        self.assertEqual(self._head(work), built["trusted"])

        self._ci(work, "fetch", self._ci_env(built, pull="10", head=built["grown"]))
        self.assertEqual(self._head(work), built["trusted"])
        grown = self._gate(work, built, head=built["grown"], pull="10", labels="")
        self.assertEqual(grown.returncode, 1, grown.stderr)
        self.assertIn("baseline-growth-approved", grown.stderr)
        self.assertIn("# owner:", grown.stderr)
        self.assertFalse(marker.exists())
        self.assertEqual(self._head(work), built["trusted"])
        self.assertIn("proposal", (work / "scripts" / "lint-baseline-growth.py").read_text(encoding="utf-8"))

        self._ci(work, "fetch", self._ci_env(built, pull="11", head=built["owned"]))
        self.assertEqual(self._head(work), built["trusted"])
        owned = self._gate(
            work,
            built,
            head=built["owned"],
            pull="11",
            labels="baseline-growth-approved",
        )
        self.assertEqual(owned.returncode, 0, owned.stderr)
        self.assertIn("owner lines present", owned.stdout)
        self.assertFalse(marker.exists())

    def test_fetch_mismatch_bad_ref_and_missing_script_fail_closed(self) -> None:
        marker = self.repo / "untrusted-ran"
        built = self._trusted_remote(marker)
        work = Path(built["work"])
        mismatch = self._ci(
            work,
            "fetch",
            self._ci_env(built, pull="9", head=built["base"]),
        )
        self.assertEqual(mismatch.returncode, 2, mismatch.stdout)
        self.assertIn("event head", mismatch.stderr)
        self.assertEqual(self._head(work), built["trusted"])
        self.assertFalse(marker.exists())

        for name in ("older;touch pwn", "../older", "--upload-pack=evil", "older:refs/heads/main", "refs/heads/older"):
            rejected = self._ci(work, "fetch", self._ci_env(built, pull="9", head=built["same"], base_ref=name))
            self.assertEqual(rejected.returncode, 2, name)
            self.assertIn("base ref", rejected.stderr)
            self.assertEqual(self._head(work), built["trusted"])
            self.assertFalse(marker.exists())
            self.assertFalse((work / "pwn").exists())

        missing_object = self._ci(
            work,
            "fetch",
            self._ci_env(built, pull="9", head=built["same"], base_sha="a" * 40),
        )
        self.assertEqual(missing_object.returncode, 2, missing_object.stdout)
        self.assertIn("cannot fetch base sha", missing_object.stderr)
        self.assertEqual(self._head(work), built["trusted"])

        script = work / "scripts" / "lint-baseline-growth.py"
        script.unlink()
        absent = self._ci(work, "guard", self._ci_env(built, pull="9", head=built["same"]))
        self.assertEqual(absent.returncode, 2, absent.stdout)
        self.assertIn("no gate script", absent.stderr)
        outside = self.repo / "outside.py"
        outside.write_text("raise SystemExit(0)\n", encoding="utf-8")
        script.symlink_to(outside)
        linked = self._ci(work, "guard", self._ci_env(built, pull="9", head=built["same"]))
        self.assertEqual(linked.returncode, 2, linked.stdout)
        self.assertIn("resolves outside", linked.stderr)
        self.assertFalse(marker.exists())
        self.assertEqual(self._head(work), built["trusted"])

    def test_api_failure_fails_closed_for_growth_on_an_old_base(self) -> None:
        marker = self.repo / "untrusted-ran"
        built = self._trusted_remote(marker)
        work = Path(built["work"])
        self._ci(work, "fetch", self._ci_env(built, pull="10", head=built["grown"]))
        self.assertEqual(self._head(work), built["trusted"])
        down = self._gate(work, built, head=built["grown"], pull="10", token="", api="http://127.0.0.1:1")
        self.assertEqual(down.returncode, 2, down.stdout)
        self.assertIn("no token", down.stderr)
        self.assertFalse(marker.exists())
        url = self._serve({"message": "no"}, status=500)
        broken = self._gate(
            work,
            built,
            head=built["grown"],
            pull="10",
            token="token",
            api=url,
        )
        self.assertEqual(broken.returncode, 2, broken.stdout)
        self.assertIn("HTTP 500", broken.stderr)
        self.assertFalse(marker.exists())
        self.assertEqual(self._head(work), built["trusted"])

    def _serve(self, payload: dict, status: int, raw: bytes | None = None) -> str:
        body = raw if raw is not None else json.dumps(payload).encode()

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802
                self.send_response(status)
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, fmt: str, *args) -> None:
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.shutdown)
        self.addCleanup(server.server_close)
        return f"http://127.0.0.1:{server.server_address[1]}"

    def _fetch_stack(self) -> Path:
        fetched = self.repo / "stack"
        fetched.mkdir()
        subprocess.run(["git", "init", "-q"], cwd=fetched, check=True, env=git_env())
        proc = subprocess.run(
            ["git", "fetch", "--no-tags", STACK_REMOTE, STACK_BASE, STACK_HEAD],
            cwd=fetched,
            check=False,
            capture_output=True,
            text=True,
            env=git_env(),
            timeout=60,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return fetched

    def _run_in(self, repo: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
        env = git_env()
        for name in ("GITHUB_TOKEN", "GH_TOKEN", "GENOS_BASELINE_GROWTH_LABEL", "GITHUB_API_URL"):
            env.pop(name, None)
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            cwd=repo,
            check=False,
            text=True,
            capture_output=True,
            env=env,
        )

    def _commit_baselines(self, repo: Path, parent: str, files: dict[str, str]) -> str:
        env = git_env()

        def git(*args: str, input_bytes: bytes | None = None) -> str:
            proc = subprocess.run(
                ["git", *args],
                cwd=repo,
                check=True,
                capture_output=True,
                env=env,
                input=input_bytes,
            )
            return proc.stdout.decode("utf-8").strip()

        listed = git("ls-tree", "-r", "--name-only", parent, "lint").splitlines()
        baselines = [path for path in listed if Path(path).name.endswith(".txt") and "baseline" in Path(path).name]
        index = repo / ".git" / "stack-index"
        indexed = git_env({"GIT_INDEX_FILE": str(index)})

        def git_index(*args: str, input_bytes: bytes | None = None) -> str:
            proc = subprocess.run(
                ["git", *args],
                cwd=repo,
                check=True,
                capture_output=True,
                env=indexed,
                input=input_bytes,
            )
            return proc.stdout.decode("utf-8").strip()

        git_index("read-tree", "--empty")
        for path in baselines:
            if path in files:
                blob = git_index("hash-object", "-w", "--stdin", input_bytes=files[path].encode())
            else:
                blob = git("rev-parse", f"{parent}:{path}")
            git_index("update-index", "--add", "--cacheinfo", f"100644,{blob},{path}")
        tree = git_index("write-tree")
        return git(
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit-tree",
            tree,
            "-p",
            parent,
            "-m",
            "probe",
        )

    def _trusted_remote(self, marker: Path) -> dict[str, str]:
        seed = self.repo / "seed"
        bare = self.repo / "origin.git"
        work = self.repo / "work"
        seed.mkdir()
        env = git_env()

        def git(cwd: Path, *args: str) -> str:
            proc = subprocess.run(
                ["git", *args],
                cwd=cwd,
                check=True,
                capture_output=True,
                text=True,
                env=env,
            )
            return proc.stdout.strip()

        def commit(message: str, files: dict[str, str]) -> str:
            for name, text in files.items():
                path = seed / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
                git(seed, "add", "--", name)
            git(seed, "commit", "-q", "-m", message)
            return git(seed, "rev-parse", "HEAD")

        git(seed, "init", "-q", "-b", "main")
        git(seed, "config", "user.email", "test@example.com")
        git(seed, "config", "user.name", "Test")
        git(seed, "config", "commit.gpgsign", "false")
        base_files = {LOSSY: "crates/a.rs 1\n", SHADER: "crates/render/shaders/a.comp 1\n"}
        git(seed, "checkout", "-q", "--orphan", "older")
        base = commit("base without the gate", base_files)
        malicious_py = (
            "from pathlib import Path\n"
            f"Path({str(marker)!r}).write_text('ran')\n"
            "raise SystemExit(0)\n"
        )
        malicious_sh = f"#!/bin/bash\nprintf ran > {marker}\nexit 0\n"
        poisoned = {
            "scripts/lint-baseline-growth.py": malicious_py,
            "scripts/lint-baseline-growth-ci.sh": malicious_sh,
        }
        git(seed, "checkout", "-q", "--detach", base)
        same = commit("same ceilings, malicious scripts", {**base_files, **poisoned})
        git(seed, "checkout", "-q", "--detach", base)
        grown = commit(
            "growth without owner",
            {**base_files, LOSSY: "crates/a.rs 1\ncrates/b.rs 4\n", **poisoned},
        )
        git(seed, "checkout", "-q", "--detach", base)
        owned = commit(
            "growth with owner",
            {**base_files, LOSSY: f"crates/a.rs 1\n{OWNER}\ncrates/b.rs 4\n", **poisoned},
        )
        git(seed, "checkout", "-q", "--orphan", "main")
        git(seed, "rm", "-rf", "--quiet", ".")
        trusted_files = dict(base_files)
        for relative in (
            "scripts/lint-baseline-growth.py",
            "scripts/lint-baseline-growth-ci.sh",
            "scripts/lint-clippy.py",
            "scripts/lint-shader-constants.py",
        ):
            trusted_files[relative] = (ROOT / relative).read_text(encoding="utf-8")
        trusted = commit("trusted gate", trusted_files)
        subprocess.run(["git", "init", "--bare", "-q", str(bare)], check=True, env=env)
        git(bare, "config", "uploadpack.allowReachableSHA1InWant", "true")
        git(bare, "config", "uploadpack.allowAnySHA1InWant", "true")
        git(seed, "remote", "add", "origin", str(bare))
        git(seed, "push", "-q", "origin", "main", "older")
        git(seed, "push", "-q", "origin", f"{same}:refs/heads/same", f"{grown}:refs/heads/grown", f"{owned}:refs/heads/owned")
        git(bare, "update-ref", "refs/pull/9/head", same)
        git(bare, "update-ref", "refs/pull/10/head", grown)
        git(bare, "update-ref", "refs/pull/11/head", owned)
        git(bare, "symbolic-ref", "HEAD", "refs/heads/main")
        subprocess.run(["git", "clone", "-q", str(bare), str(work)], check=True, env=env, capture_output=True)
        refs = git(work, "show-ref")
        return {
            "work": str(work),
            "trusted": trusted,
            "base": base,
            "same": same,
            "grown": grown,
            "owned": owned,
            "refs": refs,
        }

    def _ci_env(
        self,
        built: dict[str, str],
        pull: str,
        head: str,
        base_ref: str = "older",
        base_sha: str | None = None,
    ) -> dict[str, str]:
        return git_env(
            {
                "GIT_LFS_SKIP_SMUDGE": "1",
                "GIT_TERMINAL_PROMPT": "0",
                "PYTHONDONTWRITEBYTECODE": "1",
                "TRUSTED_SHA": built["trusted"],
                "BASE_SHA": base_sha or built["base"],
                "BASE_REF": base_ref,
                "PR_NUMBER": pull,
                "HEAD_SHA": head,
            }
        )

    def _ci(self, work: Path, cmd: str, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(work / "scripts" / "lint-baseline-growth-ci.sh"), cmd],
            cwd=work,
            check=False,
            capture_output=True,
            text=True,
            env=env,
        )

    def _head(self, work: Path) -> str:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=work,
            check=True,
            capture_output=True,
            text=True,
            env=git_env(),
        ).stdout.strip()

    def _refs(self, work: Path) -> str:
        return subprocess.run(
            ["git", "show-ref"],
            cwd=work,
            check=True,
            capture_output=True,
            text=True,
            env=git_env(),
        ).stdout.strip()

    def _gate(
        self,
        work: Path,
        built: dict[str, str],
        head: str,
        pull: str,
        labels: str | None = None,
        token: str | None = None,
        api: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        env = self._ci_env(built, pull, head)
        env["GITHUB_REPOSITORY"] = "example/genos"
        if token is not None:
            env["GITHUB_TOKEN"] = token
        if api is not None:
            env["GITHUB_API_URL"] = api
        guard = self._ci(work, "guard", env)
        self.assertEqual(guard.returncode, 0, guard.stderr)
        args = [
            "python3",
            "scripts/lint-baseline-growth.py",
            "--base-sha",
            built["base"],
            "--head-sha",
            head,
            "--repo",
            "example/genos",
            "--pull",
            pull,
            "--label",
            "baseline-growth-approved",
        ]
        if labels is not None:
            args.extend(["--labels", labels])
        return subprocess.run(args, cwd=work, check=False, capture_output=True, text=True, env=env)

    def _run(self, args: list[str], env: dict[str, str]) -> subprocess.CompletedProcess[str]:
        merged = os.environ.copy()
        for name in ("GITHUB_TOKEN", "GH_TOKEN", "GENOS_BASELINE_GROWTH_LABEL", "GITHUB_API_URL"):
            merged.pop(name, None)
        merged.update(env)
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            cwd=self.repo,
            check=False,
            text=True,
            capture_output=True,
            env=merged,
        )

    @staticmethod
    def _boom() -> set[str]:
        raise AssertionError("metadata fetch is not used for this decision")


if os.environ.get("GENOS_BASELINE_GROWTH_LIVE_STACK") == "1":
    GitGuardTest.test_actual_stack_175_base_lacks_the_script_and_still_gates_growth = (
        GitGuardTest._live_stack_175
    )


if __name__ == "__main__":
    unittest.main()
