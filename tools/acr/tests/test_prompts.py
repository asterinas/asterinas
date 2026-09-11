from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from acr.core.contracts import PERSONAS
from acr.core.prompts import InstructionCompiler, PromptError, build_prompts
from acr.core.resolve import ResolvedTarget


class RepositoryInstructionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = self.root / "AGENTS.md"
        self.layout = "## Repository Layout\n\nlayout-version-one\n\n### Subdirectories\n\nNested layout details.\n"
        self.architecture = "## Architecture Notes\n\narchitecture-marker\n"
        self.path.write_text(
            "# Repository\n\nEXCLUDED_INTRO\n\n"
            + self.layout
            + "\n## Building and Running\n\nEXCLUDED_BUILD\n\n"
            + self.architecture
            + "\n## CI\n\nEXCLUDED_CI\n",
            encoding="utf-8",
        )
        environment = patch.dict(os.environ, {}, clear=True)
        environment.start()
        self.addCleanup(environment.stop)
        catalog = patch(
            "acr.core.prompts._guideline_catalog",
            side_effect=lambda root, persona: f"GUIDELINE_CATALOG persona={persona} digest=test-digest",
        )
        catalog.start()
        self.addCleanup(catalog.stop)
        self.compiler = InstructionCompiler()

    def test_all_personas_receive_only_selected_sections(self):
        for persona in PERSONAS:
            with self.subTest(persona=persona):
                instructions = self.compiler.compile_persona(persona, guideline_root=self.root)
                self.assertIn(self.layout.rstrip(), instructions.text)
                self.assertIn(self.architecture.rstrip(), instructions.text)
                for excluded in ("EXCLUDED_INTRO", "EXCLUDED_BUILD", "EXCLUDED_CI"):
                    self.assertNotIn(excluded, instructions.text)

    def test_combined_review_includes_repository_instructions_once(self):
        instructions = self.compiler.compile_combined(
            ("development", "security"), guideline_root=self.root
        )
        self.assertEqual(instructions.text.count(self.layout.rstrip()), 1)
        self.assertEqual(instructions.text.count(self.architecture.rstrip()), 1)

    def test_source_updates_change_instructions_and_hash(self):
        original = self.compiler.compile_persona("development", guideline_root=self.root)
        self.path.write_text(
            self.path.read_text(encoding="utf-8").replace("layout-version-one", "layout-version-two"),
            encoding="utf-8",
        )
        updated = self.compiler.compile_persona("development", guideline_root=self.root)
        self.assertIn("layout-version-two", updated.text)
        self.assertNotIn("layout-version-one", updated.text)
        self.assertNotEqual(original.sha256, updated.sha256)

    def test_explicit_guideline_source_takes_precedence_over_review_target(self):
        target = self.root / "historical-checkout"
        target.mkdir()
        (target / "AGENTS.md").write_text(
            "## Repository Layout\nTARGET_STALE_LAYOUT\n## Architecture Notes\nTARGET_STALE_ARCHITECTURE\n",
            encoding="utf-8",
        )
        with patch.dict(os.environ, {"ACR_GUIDELINE_ROOT": str(self.root)}):
            instructions = self.compiler.compile_persona("development", guideline_root=target)
        self.assertIn(self.layout.rstrip(), instructions.text)
        self.assertIn(self.architecture.rstrip(), instructions.text)
        self.assertNotIn("TARGET_STALE", instructions.text)

    def test_fenced_headings_do_not_end_sections_or_supply_missing_sections(self):
        layout = self.layout + "\n```markdown\n## Example heading\n```\n"
        self.path.write_text(layout + "\n" + self.architecture, encoding="utf-8")
        instructions = self.compiler.compile_persona("development", guideline_root=self.root)
        self.assertIn(layout.rstrip(), instructions.text)
        self.path.write_text(
            self.layout + "\n~~~markdown\n" + self.architecture + "~~~\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(PromptError, "missing sections: Architecture Notes"):
            self.compiler.compile_persona("development", guideline_root=self.root)

    def test_missing_agents_file_preserves_reviews_of_other_repositories(self):
        self.path.unlink()
        instructions = self.compiler.compile_persona("development", guideline_root=self.root)
        self.assertNotIn("Repository instructions from AGENTS.md:", instructions.text)

    def test_existing_agents_file_must_contain_both_sections(self):
        self.path.write_text(self.architecture, encoding="utf-8")
        with self.assertRaisesRegex(PromptError, "missing sections: Repository Layout"):
            self.compiler.compile_combined(("development", "security"), guideline_root=self.root)

    def test_repository_context_is_instruction_and_review_input_stays_separate(self):
        review_input = "REVIEW_INPUT_MARKER"
        target = ResolvedTarget(
            raw_args="files example.rs review.md",
            meta={"mode": "files"},
            canonical_input=review_input,
            reviewed_paths=("example.rs",),
            input_sha256="test-input-digest",
            repo_root=self.root,
        )
        built = build_prompts(target, ("development",))["development"]
        self.assertIn(self.layout.rstrip(), built.instructions.text)
        self.assertIn(self.architecture.rstrip(), built.instructions.text)
        self.assertNotIn(review_input, built.instructions.text)
        self.assertEqual(built.input.content, review_input)


if __name__ == "__main__":
    unittest.main()
