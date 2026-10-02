"""Tool contracts and the allowlist/timeout broker."""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass
from typing import Any, Callable, Protocol

from ..agents.protocol import ToolSpec


@dataclass(frozen=True)
class ToolResult:
    ok: bool
    value: Any = None
    error_code: str | None = None


class ToolProvider(Protocol):
    def specs(self) -> tuple[ToolSpec, ...]: ...
    async def invoke(self, name: str, arguments: dict[str, Any]) -> ToolResult: ...


class Tool(Protocol):
    def spec(self) -> ToolSpec: ...
    async def run(self, arguments: dict[str, Any]) -> ToolResult: ...


class ToolDenied(PermissionError):
    pass


class ToolBroker:
    def __init__(self, provider: ToolProvider, *, allowed: set[str] | None = None, timeout_seconds: float = 20.0, max_output_bytes: int = 1_000_000, event_hook: Callable[[dict[str, Any]], None] | None = None):
        self.provider = provider
        available = {spec.name for spec in provider.specs()}
        self.allowed = available if allowed is None else available & allowed
        self.timeout_seconds = timeout_seconds
        self.max_output_bytes = max_output_bytes
        self.event_hook = event_hook

    def specs(self) -> tuple[ToolSpec, ...]:
        return tuple(spec for spec in self.provider.specs() if spec.name in self.allowed)

    def policy(self, allowed: set[str] | None, *, event_hook: Callable[[dict[str, Any]], None] | None = None) -> "ToolBroker":
        """Return a narrowed broker without widening the provider allowlist."""
        hook = event_hook
        if self.event_hook is not None and event_hook is not None:
            def hook(event: dict[str, Any]) -> None:
                self.event_hook(event)
                event_hook(event)
        elif hook is None:
            hook = self.event_hook
        return ToolBroker(self.provider, allowed=(self.allowed & allowed) if allowed is not None else self.allowed, timeout_seconds=self.timeout_seconds, max_output_bytes=self.max_output_bytes, event_hook=hook)

    async def invoke(self, name: str, arguments: dict[str, Any]) -> ToolResult:
        if name not in self.allowed:
            raise ToolDenied(f"tool denied: {name}")
        if self.event_hook is not None:
            self.event_hook({"event": "tool.started", "tool": name, "arguments": arguments})
        try:
            result = await asyncio.wait_for(self.provider.invoke(name, arguments), timeout=self.timeout_seconds)
        except asyncio.TimeoutError:
            if self.event_hook is not None:
                self.event_hook({"event": "tool.failed", "tool": name, "error_code": "TOOL_TIMEOUT"})
            raise
        encoded = json.dumps(result.value, ensure_ascii=False, default=str).encode("utf-8")
        if len(encoded) > self.max_output_bytes:
            if self.event_hook is not None:
                self.event_hook({"event": "tool.failed", "tool": name, "error_code": "TOOL_OUTPUT_TOO_LARGE"})
            return ToolResult(False, error_code="TOOL_OUTPUT_TOO_LARGE")
        if self.event_hook is not None:
            self.event_hook({
                "event": "tool.completed",
                "tool": name,
                "ok": result.ok,
                "error_code": result.error_code,
                "output_bytes": len(encoded),
                "output": result.value,
            })
        return result

    def function_tools(self):
        """Adapt allowlisted specs to OpenAI Agents SDK ``FunctionTool``s.

        The SDK imports stay optional so fake/model-free runs do not require
        importing provider implementation details.
        """
        try:
            from agents import FunctionTool
        except ImportError as exc:  # pragma: no cover
            raise RuntimeError("openai-agents SDK is not installed") from exc

        tools = []
        for spec in self.specs():
            schema = dict(spec.input_schema)
            if schema.get("type") != "object":
                schema = {"type": "object", "properties": schema, "additionalProperties": False}

            async def invoke(_context, raw_arguments: str, *, tool_name=spec.name):
                try:
                    arguments = json.loads(raw_arguments)
                except json.JSONDecodeError:
                    return "TOOL_INVALID_INPUT: arguments are not JSON"
                result = await self.invoke(tool_name, arguments)
                if result.ok:
                    if isinstance(result.value, str):
                        return result.value
                    return json.dumps(result.value, ensure_ascii=False, default=str)
                return f"{result.error_code or 'TOOL_ERROR'}: {result.value or 'tool failed'}"

            # Responses tool names accept only ASCII letters, digits, ``_``
            # and ``-``.  Keep dotted IDs in the broker/audit layer while
            # presenting a legal, deterministic SDK name to the model.
            sdk_name = spec.name.replace(".", "_")
            description = (
                f"{spec.description} Execution limits: the call times out after "
                f"{self.timeout_seconds:g} seconds, and serialized output over "
                f"{self.max_output_bytes} bytes is rejected."
            )
            tools.append(
                FunctionTool(
                    sdk_name,
                    description,
                    schema,
                    invoke,
                    strict_json_schema=True,
                    timeout_seconds=self.timeout_seconds,
                )
            )
        return tools
