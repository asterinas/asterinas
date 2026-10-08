from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from pydantic import BaseModel

from acr.agents.pi import PiAgentBackend, PiAgentError
from acr.agents.pi.protocol import PiProtocolError, normalize_usage, validate_ready
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
                "Must never reach a Pi child.",
                {"type": "object", "additionalProperties": False},
                "write",
            ),
        )

    async def invoke(self, name, arguments):
        return ToolResult(True, "unexpected")


class PiProtocolTests(unittest.TestCase):
    def test_pi_example_config_is_valid(self):
        path = Path(__file__).resolve().parents[1] / "acr.pi.example.toml"
        config = load_config(path)
        self.assertEqual(config.backend, "pi-agent")
        self.assertTrue(config.pi.node_command)

    def test_ready_requires_all_capabilities(self):
        with self.assertRaises(PiProtocolError):
            validate_ready({"version": 1, "type": "ready", "capabilities": []})

    def test_missing_usage_is_not_fabricated(self):
        self.assertEqual(normalize_usage(None), {"reported": False})


class PiWriteBoundaryTests(unittest.IsolatedAsyncioTestCase):
    async def test_non_read_only_broker_tool_is_rejected_before_spawn(self):
        root = Path(tempfile.mkdtemp(prefix="acr-pi-write-boundary-"))
        config = RunConfig(
            backend="pi-agent",
            model="provider/model",
            review_model="provider/model",
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
            config.model,
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
