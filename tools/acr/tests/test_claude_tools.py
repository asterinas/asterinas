from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from acr.agents.claude.protocol import ClaudeAgentError
from acr.agents.claude.tools import ClaudeTools
from acr.agents.protocol import AgentRequest, ToolSpec
from acr.core.contracts import CommentsEnvelope
from acr.core.invocations import AgentInput, AgentInstructions, AgentInvocation, ToolPolicy
from acr.tools.builtin import default_provider
from acr.tools.protocol import ToolBroker


HAS_SDK = importlib.util.find_spec("claude_agent_sdk") is not None


def request_for(*names, policy=None):
    invocation = AgentInvocation(
        AgentInstructions(("development",), "Review", "test"),
        AgentInput.from_text("input"), CommentsEnvelope,
        policy or ToolPolicy(allow=tuple(names)),
    )
    return AgentRequest.from_invocation(invocation)


@unittest.skipUnless(HAS_SDK, "install ACR dependencies to test MCP adaptation")
class ClaudeToolsTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "hello.txt").write_text("hello\nworld\n")
        self.broker = ToolBroker(default_provider(self.root))
        self.events = []

    def tools(self, request, broker=None, *, enabled=True):
        return ClaudeTools(request, self.broker if broker is None else broker, enabled=enabled, event_hook=self.events.append)

    async def test_mcp_reuses_broker_and_source_path_boundary(self):
        import claude_agent_sdk
        with patch("claude_agent_sdk.create_sdk_mcp_server", wraps=claude_agent_sdk.create_sdk_mcp_server) as create:
            tools = self.tools(request_for("source.read", "source.search"))
        handlers = {tool.name: tool.handler for tool in create.call_args.kwargs["tools"]}
        result = await handlers["source_read"]({"ranges": [{"path": "hello.txt", "start": 1, "end": 2}]})
        self.assertFalse(result["isError"])
        self.assertIn("1: hello", result["content"][0]["text"])
        escaped = await handlers["source_read"]({"ranges": [{"path": "../outside", "start": 1, "end": 1}]})
        self.assertIn("escapes repository root", escaped["content"][0]["text"])
        searched = await handlers["source_search"]({"path": "hello.txt", "query": "world"})
        self.assertIn("world", searched["content"][0]["text"])
        self.assertEqual(tools.call_count, 3)
        self.assertEqual({event["tool"] for event in self.events}, {"source.read", "source.search"})

    async def test_write_network_unknown_and_colliding_tools_are_rejected(self):
        cases = [
            (request_for("web.search"), self.broker, "web.search"),
            (request_for("missing"), self.broker, "unavailable"),
            (request_for(policy=ToolPolicy(writes="workspace")), self.broker, "read-only"),
            (request_for(policy=ToolPolicy(network="allowlisted")), self.broker, "network"),
        ]
        class Provider:
            def __init__(self, specs):
                self._specs = specs
            def specs(self):
                return self._specs
        for specs, error in (
            ([ToolSpec("custom.write", "write", {"type": "object"}, "write")], "non-read-only"),
            ([ToolSpec(name, "read", {"type": "object"}) for name in ("a.b", "a_b")], "collision"),
        ):
            cases.append((request_for(*(spec.name for spec in specs)), ToolBroker(Provider(specs)), error))
        for request, broker, error in cases:
            with self.subTest(error=error), self.assertRaisesRegex(ClaudeAgentError, error):
                self.tools(request, broker)

    async def test_gate_and_init_fail_closed_even_for_read_only_native_tools(self):
        tools = self.tools(request_for("source.read"))
        for name in ("Bash", "Read", "Agent", "mcp__other__read", "mcp__acr__git_blame"):
            result = await tools.gate({"tool_name": name}, None, None)
            self.assertEqual(result["hookSpecificOutput"]["permissionDecision"], "deny")
        for name in ("StructuredOutput", "mcp__acr__source_read"):
            self.assertEqual(await tools.gate({"tool_name": name}, None, None), {})
        data = {"tools": tools.allowed_names, "apiKeySource": "ANTHROPIC_API_KEY", "mcp_servers": [{"name": "acr", "status": "connected"}]}
        tools.validate_init(data)
        for source in ("none", "ANTHROPIC_API_KEY", "apiKeyHelper"):
            tools.validate_init({**data, "apiKeySource": source})
        for extra in ({"tools": [*tools.allowed_names, "Read"]}, {"skills": ["outside"]}, {"slash_commands": ["outside"]}, {"plugins": ["outside"]}, {"mcp_servers": [{"name": "outside", "status": "connected"}]}):
            with self.subTest(extra=extra), self.assertRaises(ClaudeAgentError):
                tools.validate_init({**data, **extra})

    async def test_init_accepts_builtin_plugins_by_origin(self):
        tools = self.tools(request_for("source.read"))
        data = {"tools": tools.allowed_names, "mcp_servers": [{"name": "acr", "status": "connected"}]}
        plugins = [
            {"name": name, "path": "builtin", "source": f"{name}@builtin"}
            for name in ("cc-plugin-agents-md", "cc-plugin-plugin-authoring", "future-builtin")
        ]
        tools.validate_init({**data, "plugins": plugins})
        # Older CLIs omit the optional source field.
        tools.validate_init({**data, "plugins": [{"name": "legacy-builtin", "path": "builtin"}]})
        for extra in ({"skills": ["outside"]}, {"slash_commands": ["outside"]}, {"tools": [*tools.allowed_names, "Read"]}):
            with self.subTest(extra=extra), self.assertRaises(ClaudeAgentError):
                tools.validate_init({**data, "plugins": plugins, **extra})

    async def test_init_rejects_external_or_malformed_plugins(self):
        tools = self.tools(request_for("source.read"))
        data = {"tools": tools.allowed_names, "mcp_servers": [{"name": "acr", "status": "connected"}]}
        builtin = {"name": "builtin-plugin", "path": "builtin", "source": "builtin-plugin@builtin"}
        external = {"name": "external", "path": "/plugins/external", "source": "external@github"}
        for plugins in (
            [external],
            [{"name": "official", "path": "/plugins/official", "source": "official@claude-plugins-official"}],
            [{**builtin, "path": "/plugins/builtin-plugin"}],
            [{**builtin, "source": "builtin-plugin@marketplace"}],
            [{**builtin, "source": "another-plugin@builtin"}],
            [{**builtin, "source": None}],
            [builtin, external],
            [{**builtin, "name": ""}],
            [{**builtin, "name": None}],
            [{}],
            [None],
            ["outside"],
            None,
            {},
            "builtin",
        ):
            with self.subTest(plugins=plugins), self.assertRaisesRegex(ClaudeAgentError, "plugin"):
                tools.validate_init({**data, "plugins": plugins})

    async def test_disabled_tools_and_broker_limits_are_preserved(self):
        tools = self.tools(request_for("source.read"), enabled=False)
        self.assertEqual(tools.servers, {})
        self.assertEqual(tools.allowed_names, ["StructuredOutput"])
        import claude_agent_sdk
        policy = ToolPolicy(allow=("source.read",), max_output_bytes=10)
        with patch("claude_agent_sdk.create_sdk_mcp_server", wraps=claude_agent_sdk.create_sdk_mcp_server) as create:
            self.tools(request_for(policy=policy))
        handler = create.call_args.kwargs["tools"][0].handler
        result = await handler({"ranges": [{"path": "hello.txt", "start": 1, "end": 2}]})
        self.assertTrue(result["isError"])
        self.assertIn("TOOL_OUTPUT_TOO_LARGE", result["content"][0]["text"])
        self.assertEqual(self.broker.max_output_bytes, 1_000_000)
