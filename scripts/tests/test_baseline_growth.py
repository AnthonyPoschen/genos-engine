#!/usr/bin/env python3
"""Regression tests for the proposed baseline-growth gate.

The label name and the owner line are a proposal. These tests lock that
contract. They do not create a GitHub label and they do not edit the
repository baselines.
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
WORKFLOW = ROOT / ".github" / "workflows" / "baseline-growth.yml"
LOSSY = "lint/lossy-cast-baseline.txt"
SHADER = "lint/shader-magic-baseline.txt"
OWNER = "# owner: Ada Lovelace — the cast is a counted GPU size"
EM = "\u2014"


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
        self.assertNotRegex(text, r"ref:\s*\$\{\{\s*github\.event\.pull_request\.head")
        self.assertNotIn("head.ref", text)
        self.assertIn(self.guard.PROPOSED_LABEL, text)
        self.assertIn("--label", text)
        self.assertNotIn("pull_request_target", (ROOT / ".github" / "workflows" / "lint.yml").read_text(encoding="utf-8"))

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


if __name__ == "__main__":
    unittest.main()
