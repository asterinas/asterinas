from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from pydantic import BaseModel

from acr.agents.pi import PiAgentBackend, PiAgentError
from acr.agents.pi.protocol import REQUIRED_CAPABILITIES, PiProtocolError, normalize_usage, validate_ready
from acr.agents.protocol import AgentRequest, ToolSpec
from acr.config import PiConfig, RunConfig, load_config
from acr.core.invocations import (
    AgentInput,
    AgentInstructions,
    AgentInvocation,
    RunLimits,
    ToolPolicy,
)
from acr.tools.protocol import ToolBroker, ToolResult


class _Output(BaseModel):
    ok: bool


class _WriteProvider:
    def specs(self):
        return (
            ToolSpec(
                "custom.write",
                "Must never reach a Pi session.",
                {"type": "object", "additionalProperties": False},
                "write",
            ),
        )

    async def invoke(self, name, arguments):
        return ToolResult(True, "unexpected")


class PiProtocolTests(unittest.TestCase):
    def test_pi_config_is_valid(self):
        path = Path(__file__).resolve().parents[1] / "acr.pi.toml"
        config = load_config(path)
        self.assertEqual(config.backend, "pi-agent")
        self.assertTrue(config.pi.node_command)

    def test_ready_requires_all_capabilities(self):
        with self.assertRaises(PiProtocolError):
            validate_ready({"version": 1, "type": "ready", "capabilities": []})

    def test_direct_session_handshake_requires_no_delegation_capabilities(self):
        validate_ready({"version": 1, "type": "ready", "capabilities": sorted(REQUIRED_CAPABILITIES)})
        self.assertIn("agent_session", REQUIRED_CAPABILITIES)
        self.assertIn("structured_output", REQUIRED_CAPABILITIES)

    def test_missing_token_counts_are_not_fabricated(self):
        usage = normalize_usage({"turns": 1})
        self.assertIsNone(usage["input"])
        self.assertIsNone(usage["output"])
        self.assertIsNone(usage["cost"])
        self.assertEqual(usage["turns"], 1)

    def test_missing_usage_is_not_fabricated(self):
        self.assertEqual(normalize_usage(None), {"reported": False})


class PiWriteBoundaryTests(unittest.IsolatedAsyncioTestCase):
    async def test_non_read_only_broker_tool_is_rejected_before_spawn(self):
        temp = tempfile.TemporaryDirectory(prefix="acr-pi-write-boundary-")
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        config = RunConfig(
            backend="pi-agent",
            review_model="provider/review",
            postprocess_model="provider/postprocess",
            tools_enabled=True,
            pi=PiConfig(node_command="does-not-exist"),
        )
        invocation = AgentInvocation(
            AgentInstructions(("summary",), "Return structured output.", "test-v1"),
            AgentInput.from_text("Return ok=true."),
            _Output,
            ToolPolicy(allow=("custom.write",)),
            RunLimits(timeout_seconds=1),
            "summary",
            None,
            config.postprocess_model,
        )
        backend = PiAgentBackend(
            config,
            repo_root=root,
            run_root=root,
            broker=ToolBroker(_WriteProvider()),
        )
        with self.assertRaisesRegex(PiAgentError, "non-read-only"):
            await backend.run(AgentRequest.from_invocation(invocation))
        self.assertFalse((root / "pi-sessions").exists())

    async def test_native_and_output_tool_names_are_reserved(self):
        temp = tempfile.TemporaryDirectory(prefix="acr-pi-reserved-tools-")
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        config = RunConfig(backend="pi-agent", pi=PiConfig(node_command="does-not-exist"))
        for name in ("read", "bash", "subagent", "structured_output"):
            with self.subTest(name=name):
                class Provider:
                    def specs(self):
                        return (ToolSpec(name, "Reserved tool.", {"type": "object"}, "read"),)

                invocation = AgentInvocation(
                    AgentInstructions(("summary",), "Return structured output.", "test-v1"),
                    AgentInput.from_text("Return ok=true."), _Output,
                    ToolPolicy(allow=(name,)), RunLimits(timeout_seconds=1),
                    "summary", None, config.postprocess_model,
                )
                backend = PiAgentBackend(config, repo_root=root, run_root=root, broker=ToolBroker(Provider()))
                with self.assertRaisesRegex(PiAgentError, "non-read-only"):
                    await backend.run(AgentRequest.from_invocation(invocation))
        self.assertFalse((root / "pi-sessions").exists())
