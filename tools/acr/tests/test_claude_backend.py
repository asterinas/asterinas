from __future__ import annotations

import asyncio
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from acr.agents.backend_factory import create_backend, close_backend
from acr.agents.claude.backend import ClaudeAgentBackend
from acr.agents.claude.cli import find_cli
from acr.agents.claude.protocol import ClaudeAgentError, normalize_usage
from acr.agents.factories import AgentFactory
from acr.agents.protocol import AgentRequest, CancellationToken
from acr.agents.retry import run_with_retries
from acr.config import ConfigError, RunConfig, load_config
from acr.core.contracts import (
    CommentsEnvelope,
)
from acr.core.events import EventLogger
from acr.core.invocations import AgentInput, AgentInstructions
from acr.agents.tracing import logger_sink


HAS_SDK = importlib.util.find_spec("claude_agent_sdk") is not None
if HAS_SDK:
    import anyio
    from claude_agent_sdk import AssistantMessage, ResultMessage, SystemMessage, TextBlock, ThinkingBlock


class ClaudeConfigTests(unittest.TestCase):
    def test_missing_usage_is_not_reported_as_zero(self):
        result = SimpleNamespace(usage={}, total_cost_usd=None, num_turns=1, duration_api_ms=0)
        usage = normalize_usage(result, tool_calls=0)
        self.assertFalse(usage["reported"])
        for name in ("input", "output", "cache_read", "cache_write", "total", "cost"):
            self.assertIsNone(usage[name])
        result.usage = {"input_tokens": 1, "output_tokens": 2}
        usage = normalize_usage(result, tool_calls=0)
        self.assertTrue(usage["reported"])
        self.assertIsNone(usage["total"])

    def test_fake_review_does_not_import_either_sdk(self):
        source = Path(__file__).resolve().parents[1]
        script = '''
import importlib.abc
import runpy
import sys
class RejectSDK(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname == "agents" or fullname.startswith("claude_agent_sdk"):
            raise ImportError("SDK is deliberately unavailable")
sys.meta_path.insert(0, RejectSDK())
bootstrap, config, output = sys.argv[1:]
sys.argv = [bootstrap, "acr.run_review", "--config", config, "--backend=fake", "files", "README.md", output]
runpy.run_path(bootstrap, run_name="__main__")
'''
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "review.md"
            result = subprocess.run(
                [sys.executable, "-c", script, str(source / "src" / "_bootstrap.py"), str(source / "acr.claude.toml"), str(output)],
                cwd=source.parents[1], capture_output=True, text=True, timeout=20,
                env={**os.environ, "ACR_LOG_ROOT": str(Path(directory) / "logs")},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(output.is_file())

    def test_profile_inherits_the_cli_model_without_claude_settings(self):
        with patch.dict(os.environ, {}, clear=True):
            config = load_config(Path(__file__).resolve().parents[1] / "acr.claude.toml")
        self.assertEqual(config.backend, "claude-agent")
        self.assertEqual(config.review_model, "default")
        self.assertEqual(config.postprocess_model, "default")
        self.assertFalse(hasattr(config, "claude"))

    def test_removed_authentication_configuration_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "acr.toml"
            path.write_text('[agent]\nbackend="claude-agent"\n[claude]\nauth_mode="api_key"\n')
            with self.assertRaisesRegex(ConfigError, "remove the \\[claude\\] table"):
                load_config(path)

    def test_configuration_does_not_require_credentials_or_a_cli(self):
        with patch.dict(os.environ, {}, clear=True):
            RunConfig(backend="fake").validate()
            RunConfig(backend="claude-agent").validate()

    def test_cli_is_selected_from_path_without_a_bundled_fallback(self):
        with patch("acr.agents.claude.cli.shutil.which", return_value="/local/bin/claude"):
            self.assertEqual(find_cli(), "/local/bin/claude")
        with patch("acr.agents.claude.cli.shutil.which", return_value=None):
            with self.assertRaisesRegex(ClaudeAgentError, "not found on PATH"):
                find_cli()


_DEFAULT_OUTPUT = object()


class FakeClient:
    def __init__(self, options, *, output=_DEFAULT_OUTPUT, subtype="success", is_error=False, block=False, connect_block=False, api_key_source="none"):
        self.options = options
        self.output = {"comments": []} if output is _DEFAULT_OUTPUT else output
        self.subtype = subtype
        self.is_error = is_error
        self.block = block
        self.connect_block = connect_block
        self.api_key_source = api_key_source
        self.started = asyncio.Event()
        self.disconnected = False
        self.interrupted = False
        self.prompt = None
        self.messages = []

    async def connect(self):
        if self.connect_block:
            self.started.set()
            await anyio.sleep_forever()

    async def query(self, prompt):
        self.prompt = prompt

    async def receive_response(self):
        self.started.set()
        if self.block:
            await anyio.sleep_forever()
        if self.messages:
            for message in self.messages:
                yield message
            return
        yield SystemMessage("init", {
            "tools": self.options.allowed_tools,
            "apiKeySource": self.api_key_source,
            "mcp_servers": [{"name": name, "status": "connected"} for name in self.options.mcp_servers],
            "claude_code_version": "test", "model": self.options.model,
        })
        yield ResultMessage(
            subtype=self.subtype, duration_ms=10, duration_api_ms=5,
            is_error=self.is_error, num_turns=1, session_id="test-session",
            usage={"input_tokens": 3, "output_tokens": 2, "cache_read_input_tokens": 4, "cache_creation_input_tokens": 1},
            total_cost_usd=0.01, structured_output=self.output,
        )

    async def interrupt(self):
        self.interrupted = True

    async def disconnect(self):
        await anyio.sleep(0.01)
        self.disconnected = True


@unittest.skipUnless(HAS_SDK, "install ACR dependencies to test the SDK adapter")
class ClaudeBackendTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = patch.dict(os.environ, {"ANTHROPIC_API_KEY": "test-secret"}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        self.config = RunConfig(
            backend="claude-agent", review_model="review-model", postprocess_model="post-model",
            reasoning_effort="high", postprocess_reasoning_effort="low",
        )
        self.cli = patch("acr.agents.claude.backend.find_cli", return_value="/local/bin/claude")
        self.cli.start()
        self.addCleanup(self.cli.stop)
        self.backend = create_backend(self.config, repo_root=self.root, run_root=self.root)
        self.addAsyncCleanup(close_backend, self.backend)
        self.request = AgentRequest("persona", "stable", "volatile", CommentsEnvelope, model="review-model")
        self.clients = []

    def factory(self, **settings):
        def create(*, options):
            client = FakeClient(options, **settings)
            self.clients.append(client)
            return client
        return create

    async def test_output_contracts_and_model_selection_use_existing_factories(self):
        factory = AgentFactory(self.config)
        instructions = AgentInstructions(("development",), "stable", "test")
        invocations = [factory.persona(instructions, AgentInput.from_text("volatile"), persona="development")]
        invocations.extend(getattr(factory, role)("input") for role in ("verification", "consolidation", "summary", "grader"))
        outputs = ({"comments": []}, {"items": []}, {"shared_fixes": {"root": "fix"}, "comment_updates": {}}, {"summary": "done"}, {"results": []})
        for invocation, output in zip(invocations, outputs):
            with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=self.factory(output=output)):
                response = await self.backend.run(AgentRequest.from_invocation(invocation))
            client = self.clients[-1]
            self.assertEqual(response.value, output)
            self.assertEqual(client.options.model, self.config.model_for(invocation.role))
            self.assertEqual(client.options.system_prompt, invocation.instructions.text)
            self.assertEqual(client.prompt, invocation.input.content)
            self.assertTrue(client.disconnected)
            self.assertEqual(client.options.effort, "high" if invocation.role == "persona" else "low")
            self.assertEqual(client.options.cli_path, "/local/bin/claude")
            self.assertEqual(client.options.cwd, self.root)
            self.assertEqual(client.options.env, {})
            self.assertEqual(client.options.setting_sources, ["user", "project", "local"])
            self.assertIn("safe-mode", client.options.extra_args)
            self.assertFalse(client.options.continue_conversation)
            self.assertIsNone(client.options.resume)
            self.assertEqual(response.usage["total"], 10)
        self.assertEqual(len({id(client) for client in self.clients}), 5)

    async def test_default_model_and_authentication_are_resolved_by_the_cli(self):
        cases = (
            ({"CLAUDE_CODE_OAUTH_TOKEN": "subscription-token"}, "none"),
            ({"ANTHROPIC_API_KEY": "api-key"}, "ANTHROPIC_API_KEY"),
            ({"ANTHROPIC_AUTH_TOKEN": "relay-token", "ANTHROPIC_BASE_URL": "https://relay.invalid"}, "none"),
            ({}, "apiKeyHelper"),
        )
        request = replace(self.request, model="default")
        for environment, source in cases:
            with self.subTest(source=source), patch.dict(os.environ, environment, clear=True):
                before = dict(os.environ)
                with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=self.factory(api_key_source=source)):
                    response = await self.backend.run(request)
                self.assertEqual(response.value, {"comments": []})
                self.assertIsNone(self.clients[-1].options.model)
                self.assertEqual(self.clients[-1].options.env, {})
                self.assertEqual(dict(os.environ), before)

    async def test_invalid_output_retries_with_new_sessions_and_records_failed_usage(self):
        logger = EventLogger(self.root, "test")
        def create(*, options):
            client = FakeClient(options, output={"wrong": True} if not self.clients else {"comments": []})
            self.clients.append(client)
            return client
        with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=create):
            await run_with_retries(lambda: self.backend.run(self.request, event_sink=logger_sink(logger, stage="persona")), 1)
        aggregate = json.loads((self.root / "usage.json").read_text())
        self.assertEqual(aggregate["invocation_count"], 2)
        self.assertEqual(aggregate["totals"]["cost"], 0.02)
        self.assertEqual(len(self.clients), 2)
        self.assertIsNot(self.clients[0], self.clients[1])
        self.assertTrue(all(client.disconnected for client in self.clients))

    async def test_success_without_structured_output_and_error_results_fail(self):
        for output, subtype, is_error in ((None, "success", False), (False, "success", False), ({}, "error_during_execution", True), ({}, "success", True)):
            with self.subTest(subtype=subtype, is_error=is_error):
                create = self.factory(output=output, subtype=subtype, is_error=is_error)
                with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=create), self.assertRaises(ClaudeAgentError):
                    await self.backend.run(self.request)
        client = FakeClient(None)
        async def stream():
            yield ResultMessage("success", 0, 0, False, 1, "s", structured_output=None)
        client.receive_response = stream
        with self.assertRaisesRegex(ClaudeAgentError, "before reporting its tools"):
            await self.backend._receive(client, None, lambda *args, **kwargs: None)

    async def test_token_task_timeout_and_close_all_wait_for_cleanup(self):
        for mode in ("token", "task", "timeout", "close", "startup"):
            with self.subTest(mode=mode):
                self.clients.clear()
                backend = ClaudeAgentBackend(self.config, repo_root=self.root)
                token = CancellationToken()
                request = replace(self.request, timeout_seconds=0.05 if mode == "timeout" else None)
                with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=self.factory(block=True, connect_block=mode == "startup")):
                    task = asyncio.create_task(backend.run(request, cancellation=token))
                    while not self.clients or not self.clients[-1].started.is_set():
                        await asyncio.sleep(0.001)
                    client = self.clients[-1]
                    await client.started.wait()
                    if mode in {"token", "startup"}:
                        token.cancel()
                    elif mode == "task":
                        task.cancel()
                    elif mode == "close":
                        await backend.aclose()
                    with self.assertRaises(ClaudeAgentError if mode == "timeout" else asyncio.CancelledError):
                        await asyncio.wait_for(task, 2)
                    self.assertTrue(client.disconnected)
                    self.assertEqual(client.interrupted, mode != "startup")
                    await backend.aclose()
                    self.assertFalse(backend._sessions)

    async def test_extra_tools_fail_before_any_result_is_accepted(self):
        create = self.factory()
        def malicious(*, options):
            client = create(options=options)
            client.messages = [SystemMessage("init", {"tools": ["Bash"], "apiKeySource": "ANTHROPIC_API_KEY"})]
            return client
        with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=malicious), self.assertRaisesRegex(ClaudeAgentError, "unexpected tools"):
            await self.backend.run(self.request)
        self.assertTrue(self.clients[-1].disconnected)

    async def test_raw_thinking_is_not_logged(self):
        backend = ClaudeAgentBackend(self.config, repo_root=self.root)
        events = []
        def create(*, options):
            client = FakeClient(options)
            original = client.receive_response
            async def stream():
                async for message in original():
                    if isinstance(message, ResultMessage):
                        yield AssistantMessage([ThinkingBlock("private thinking", "sig"), TextBlock("visible")], "test")
                    yield message
            client.receive_response = stream
            return client
        with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=create):
            await backend.run(self.request, event_sink=events.append)
        self.assertNotIn("private thinking", str(events))
        self.assertIn("visible", str(events))
        reply = next(event for event in events if event.event_type == "agent.reply" and "output" in event.data)
        self.assertEqual(reply.data["output"], {"comments": []})

    async def test_invalid_output_and_credentials_do_not_enter_failure_events(self):
        backend = ClaudeAgentBackend(self.config, repo_root=self.root)
        events = []
        def create(*, options):
            options.stderr("test-secret")
            return FakeClient(options, output={"comments": ["private invalid output"]})
        with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=create):
            with self.assertRaises(ClaudeAgentError):
                await backend.run(self.request, event_sink=events.append)
        self.assertNotIn("test-secret", str(events))
        self.assertNotIn("private invalid output", str(events))
        self.assertIn("[REDACTED]", str(events))

    async def test_inherited_bearer_token_is_redacted_from_failures(self):
        events = []
        def create(*, options):
            options.stderr("relay-secret")
            raise RuntimeError("relay-secret")
        with patch.dict(os.environ, {"ANTHROPIC_AUTH_TOKEN": "relay-secret"}, clear=True):
            with patch("claude_agent_sdk.ClaudeSDKClient", side_effect=create):
                with self.assertRaises(ClaudeAgentError) as error:
                    await self.backend.run(self.request, event_sink=events.append)
        self.assertNotIn("relay-secret", str(error.exception))
        self.assertNotIn("relay-secret", str(events))


if __name__ == "__main__":
    unittest.main()
