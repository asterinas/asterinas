from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path

from acr.config import RunConfig
from acr.core.state import RunContext, Stage, load_state


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
            self.assertEqual(RunContext.resume(context.root).state.run_id, "acr-test")
            self.assertEqual((context.root.stat().st_mode & 0o777), 0o700)
            self.assertEqual((path.stat().st_mode & 0o777), 0o600)
            self.assertTrue((context.root / "manifest.json").exists())


if __name__ == "__main__":
    unittest.main()
