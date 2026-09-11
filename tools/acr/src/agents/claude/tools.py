"""Expose only scoped ACR tools through the SDK's in-process MCP server."""

from __future__ import annotations

import json
import re
from typing import Any, Callable

from ...tools.protocol import ToolBroker
from ..protocol import AgentRequest
from .protocol import ClaudeAgentError


OUTPUT_TOOL = "StructuredOutput"
_READ_ONLY_CAPABILITIES = {"read", "source", "git", "guideline", "rust"}
_MAX_INLINE_RESULT_CHARS = 500_000


class ClaudeTools:
    def __init__(
        self,
        request: AgentRequest,
        broker: ToolBroker | None,
        *,
        enabled: bool,
        event_hook: Callable[[dict[str, Any]], None],
    ):
        from claude_agent_sdk import SdkMcpTool, ToolAnnotations, create_sdk_mcp_server

        self.call_count = 0
        self._event_hook = event_hook
        self.names: dict[str, str] = {}
        self.servers = {}
        allowed = (
            set(request.invocation.tool_policy.allow)
            if request.invocation is not None
            else {spec.name for spec in request.tools}
        )
        if request.invocation is not None:
            policy = request.invocation.tool_policy
            if policy.writes != "none" or policy.network != "disabled":
                raise ClaudeAgentError("Claude backend supports only read-only tools without network access")
        if not enabled:
            return
        if "web.search" in allowed:
            raise ClaudeAgentError("Claude backend does not support web.search; remove it from [tools]")
        available = {spec.name for spec in broker.specs()} if broker else set()
        missing = allowed - available
        if missing:
            raise ClaudeAgentError("unavailable ACR tools: " + ", ".join(sorted(missing)))
        if broker is None or not allowed:
            return
        self.broker = broker.policy(allowed, event_hook=event_hook)
        # CLI-spooled results require its native Read tool, which ACR disables.
        # Bound serialized bytes so every accepted result also fits the CLI's
        # maximum inline character count, including non-ASCII tool output.
        self.broker.max_output_bytes = min(self.broker.max_output_bytes, _MAX_INLINE_RESULT_CHARS)
        if request.invocation is not None:
            self.broker.timeout_seconds = min(self.broker.timeout_seconds, policy.timeout_seconds)
            self.broker.max_output_bytes = min(self.broker.max_output_bytes, policy.max_output_bytes)
        tools = []
        for spec in self.broker.specs():
            name = spec.name.replace(".", "_")
            full_name = f"mcp__acr__{name}"
            if spec.capability not in _READ_ONLY_CAPABILITIES:
                raise ClaudeAgentError(f"Claude backend rejects non-read-only tool: {spec.name}")
            if not re.fullmatch(r"[A-Za-z0-9_-]+", name) or len(full_name) > 64:
                raise ClaudeAgentError(f"invalid Claude tool name: {spec.name}")
            if full_name in self.names:
                raise ClaudeAgentError(f"Claude tool name collision: {spec.name}")
            self.names[full_name] = spec.name
            description = (
                f"{spec.description} Execution limits: {self.broker.timeout_seconds:g} seconds "
                f"and {self.broker.max_output_bytes} serialized output bytes."
            )
            tools.append(SdkMcpTool(
                name, description, spec.input_schema, self._handler(spec.name),
                annotations=ToolAnnotations(
                    readOnlyHint=True, openWorldHint=False,
                    maxResultSizeChars=_MAX_INLINE_RESULT_CHARS,
                ),
            ))
        self.servers = {"acr": create_sdk_mcp_server(name="acr", tools=tools)}

    @property
    def allowed_names(self) -> list[str]:
        return [*self.names, OUTPUT_TOOL]

    async def gate(self, data: dict[str, Any], _tool_id: str | None, _context: Any) -> dict[str, Any]:
        if data.get("tool_name") in self.allowed_names:
            return {}
        self._event_hook({"event": "tool.denied", "tool": str(data.get("tool_name")), "error_code": "TOOL_DENIED"})
        return {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "tool is not in this ACR invocation's allowlist",
            }
        }

    def validate_init(self, data: dict[str, Any]) -> None:
        names = data.get("tools")
        if not isinstance(names, list) or any(not isinstance(name, str) for name in names):
            raise ClaudeAgentError("Claude init did not report its tools")
        unexpected = set(names) - set(self.allowed_names)
        if unexpected:
            raise ClaudeAgentError("Claude exposed unexpected tools: " + ", ".join(sorted(unexpected)))
        if OUTPUT_TOOL not in names or set(self.names) - set(names):
            raise ClaudeAgentError("Claude did not expose all required ACR tools")
        if data.get("skills") or data.get("slash_commands"):
            raise ClaudeAgentError("Claude loaded unexpected skills or commands")
        plugins = data.get("plugins", [])
        if not isinstance(plugins, list) or any(not isinstance(plugin, dict) for plugin in plugins):
            raise ClaudeAgentError("Claude init reported an invalid plugin list")
        # Safe mode keeps CLI-bundled plugins; identify them by origin, not name.
        for plugin in plugins:
            name = plugin.get("name")
            expected_source = f"{name}@builtin"
            if (
                not isinstance(name, str)
                or not name
                or plugin.get("path") != "builtin"
                or plugin.get("source", expected_source) != expected_source
            ):
                raise ClaudeAgentError(
                    f"Claude loaded an unexpected plugin: name={name!r}, "
                    f"source={plugin.get('source')!r}, path={plugin.get('path')!r}"
                )
        servers = data.get("mcp_servers", [])
        if not isinstance(servers, list) or any(
            not isinstance(server, dict)
            or server.get("name") not in self.servers
            or server.get("status") != "connected"
            for server in servers
        ):
            raise ClaudeAgentError("Claude loaded an unexpected or disconnected MCP server")
        if {server["name"] for server in servers} != set(self.servers):
            raise ClaudeAgentError("Claude did not connect all required ACR MCP servers")

    def _handler(self, name: str):
        async def invoke(arguments: dict[str, Any]) -> dict[str, Any]:
            self.call_count += 1
            try:
                result = await self.broker.invoke(name, arguments)
            except (TimeoutError, PermissionError, ValueError) as exc:
                return {"content": [{"type": "text", "text": f"TOOL_ERROR: {exc}"}], "isError": True}
            text = result.value if isinstance(result.value, str) else json.dumps(result.value, ensure_ascii=False, default=str)
            if not result.ok:
                text = f"{result.error_code or 'TOOL_ERROR'}: {text}"
            return {"content": [{"type": "text", "text": text}], "isError": not result.ok}

        return invoke
