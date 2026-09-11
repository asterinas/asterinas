from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from acr.config import RunConfig
from acr.core.activation import DeterministicActivationPolicy
from acr.core.resolve import ResolveError, resolve_target
from acr.core.state import RunContext, Stage, load_state


class ResolveTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.git("init", "-q")
        self.git("commit", "--allow-empty", "-qm", "Base snapshot")

    def git(self, *args: str) -> None:
        subprocess.run(
            ["git", "-c", "user.name=ACR Test", "-c", "user.email=acr@example.invalid",
             "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
            cwd=self.root, capture_output=True, check=True,
        )

    def test_diff_activates_specialists_for_reverted_changes(self):
        paths = ("a guide.md", "boot.S")
        for path in paths:
            (self.root / path).write_text("added\n", encoding="utf-8")
        self.git("add", "--", *paths)
        self.git("commit", "-qm", "Add guide and assembly")
        self.git("revert", "--no-edit", "HEAD")
        self.git("commit", "--allow-empty", "-qm", "Commit message only")

        target = resolve_target("diff HEAD~3 out.md", repo_root=self.root)
        self.assertCountEqual(target.reviewed_paths, paths)
        self.assertEqual(
            DeterministicActivationPolicy().for_paths(target.reviewed_paths),
            ("maintainability", "development", "security", "hardware", "documentation"),
        )

    def test_diff_rejects_ranges_without_file_changes(self):
        with self.assertRaisesRegex(ResolveError, "no commits"):
            resolve_target("diff HEAD out.md", repo_root=self.root)

        self.git("commit", "--allow-empty", "-qm", "Commit message only")
        with self.assertRaisesRegex(ResolveError, "no file changes in the selected commits"):
            resolve_target("diff HEAD^ out.md", repo_root=self.root)


class StateTests(unittest.TestCase):
    def test_run_context_has_private_artifacts_and_resume_state(self):
        with tempfile.TemporaryDirectory() as temp:
            config = RunConfig(backend="fake", log_root=Path(temp))
            context = RunContext.create("files foo.rs out.md", config, run_id="acr-test")
            record = context.state.begin(Stage.RESOLVED)
            path = context.write_text("artifacts/example.txt", "hello")
            context.state.complete(record, artifact=path)
            context.save()
            self.assertEqual(load_state(context.root).stage, Stage.RESOLVED)
            resumed = RunContext.resume(context.root)
            self.assertEqual(resumed.state.run_id, "acr-test")
            self.assertNotIn("disclosure", resumed.manifest)
            self.assertEqual((context.root.stat().st_mode & 0o777), 0o700)
            self.assertEqual((path.stat().st_mode & 0o777), 0o600)
            self.assertTrue((context.root / "manifest.json").exists())


if __name__ == "__main__":
    unittest.main()
