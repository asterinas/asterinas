from __future__ import annotations

import asyncio
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from acr.agents.factories import AgentFactory, persona_request
from acr.agents.fake import FakeBackend
from acr.config import ConfigError, RunConfig, load_config
from acr.core.invocations import AgentInput, AgentInstructions
from acr.core.scheduler import PassScheduler
from acr.core.state import RunContext


class ModelSelectionTests(unittest.TestCase):
    # Profile table, RunConfig field, and environment variable of each model setting.
    STAGES = (
        ("persona_review", "review_model", "ACR_PERSONA_REVIEW_MODEL"),
        ("postprocess", "postprocess_model", "ACR_POSTPROCESS_MODEL"),
    )

    def test_model_settings_have_independent_defaults(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.toml"
            for (table, key, _), (_, other, _) in (self.STAGES, self.STAGES[::-1]):
                with self.subTest(key=key):
                    path.write_text(f'[agent.{table}]\nmodel = "custom-model"\n', encoding="utf-8")
                    with patch.dict(os.environ, {}, clear=True):
                        config = load_config(path)
                    self.assertEqual(getattr(config, key), "custom-model")
                    self.assertEqual(getattr(config, other), "gpt-5.5")

    def test_model_overrides_are_independent(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.toml"
            path.write_text(
                '[agent.persona_review]\nmodel = "review-toml"\n[agent.postprocess]\nmodel = "postprocess-toml"\n',
                encoding="utf-8",
            )
            for (table, key, env), (_, other, _) in (self.STAGES, self.STAGES[::-1]):
                with self.subTest(key=key):
                    with patch.dict(os.environ, {env: "env-model"}, clear=True):
                        config = load_config(path)
                        overridden = load_config(path, overrides={f"agent.{table}.model": "override-model"})
                    self.assertEqual(getattr(config, key), "env-model")
                    self.assertEqual(getattr(overridden, key), "override-model")
                    self.assertEqual(getattr(config, other), f"{other.removesuffix('_model')}-toml")
                    self.assertEqual(getattr(overridden, other), getattr(config, other))

    def test_empty_models_are_rejected(self):
        for table, key, _ in self.STAGES:
            with self.subTest(key=key), self.assertRaisesRegex(ConfigError, f"agent.{table}.model"):
                RunConfig(**{key: " "}).validate()

    def test_legacy_model_setting_has_a_migration_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.toml"
            path.write_text('[agent]\nmodel = "legacy-model"\n', encoding="utf-8")
            with patch.dict(os.environ, {}, clear=True):
                with self.assertRaisesRegex(ConfigError, "agent.model is no longer supported"):
                    load_config(path)
            path.write_text("", encoding="utf-8")
            with patch.dict(os.environ, {"ACR_MODEL": "legacy-model"}, clear=True):
                with self.assertRaisesRegex(ConfigError, "ACR_MODEL is no longer supported"):
                    load_config(path)

    def test_factories_select_model_by_role(self):
        config = RunConfig(review_model="review-model", postprocess_model="postprocess-model")
        factory = AgentFactory(config)
        instructions = AgentInstructions(("development",), "Review.", "test-v1")
        review_input = AgentInput.from_text("code")
        review = factory.persona(instructions, review_input, persona="development")
        combined = factory.combined(instructions, review_input, personas=("development", "security"))
        self.assertEqual(review.model, "review-model")
        self.assertEqual(combined.model, "review-model")
        self.assertEqual(persona_request("development", "code", config).model, "review-model")
        for role in ("verification", "consolidation", "summary", "grader"):
            with self.subTest(role=role):
                self.assertEqual(getattr(factory, role)("{}").model, "postprocess-model")
                self.assertEqual(config.model_for(role), "postprocess-model")

    def test_scheduler_uses_review_model_in_both_context_modes(self):
        for mode in ("yes", "no"):
            with self.subTest(mode=mode):
                config = RunConfig(
                    review_model="review-model", postprocess_model="postprocess-model",
                    per_persona_context=mode,
                )
                backend = FakeBackend()
                asyncio.run(PassScheduler(backend, config).run({"development": "code"}, ("development",)))
                self.assertEqual(backend.requests[0].model, "review-model")

    def test_manifest_records_both_models(self):
        with tempfile.TemporaryDirectory() as directory:
            config = RunConfig(
                review_model="review-model", postprocess_model="postprocess-model",
                log_root=Path(directory),
            )
            context = RunContext.create("files README.md review.md", config)
            self.assertEqual(context.manifest["review_model"], "review-model")
            self.assertEqual(context.manifest["postprocess_model"], "postprocess-model")
            self.assertNotIn("model", context.manifest)
