"""Adapt shared ACR tools into OpenAI Agents SDK function tools."""

from __future__ import annotations

import json
from typing import Any

from ...tools.protocol import ToolBroker


def function_tools(broker: ToolBroker) -> list[Any]:
    """Build SDK tools from a role-scoped broker's allowlisted definitions."""
    try:
        from agents import FunctionTool
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError("openai-agents SDK is not installed") from exc

    tools = []
    for spec in broker.specs():
        schema = dict(spec.input_schema)
        if schema.get("type") != "object":
            schema = {"type": "object", "properties": schema, "additionalProperties": False}

        async def invoke(_context, raw_arguments: str, *, tool_name=spec.name):
            try:
                arguments = json.loads(raw_arguments)
            except json.JSONDecodeError:
                return "TOOL_INVALID_INPUT: arguments are not JSON"
            result = await broker.invoke(tool_name, arguments)
            if result.ok:
                if isinstance(result.value, str):
                    return result.value
                return json.dumps(result.value, ensure_ascii=False, default=str)
            return f"{result.error_code or 'TOOL_ERROR'}: {result.value or 'tool failed'}"

        # Responses tool names accept only ASCII letters, digits, ``_``
        # and ``-``. Keep dotted IDs in the broker/audit layer while
        # presenting a legal, deterministic SDK name to the model.
        sdk_name = spec.name.replace(".", "_")
        description = (
            f"{spec.description} Execution limits: the call times out after "
            f"{broker.timeout_seconds:g} seconds, and serialized output over "
            f"{broker.max_output_bytes} bytes is rejected."
        )
        tools.append(
            FunctionTool(
                sdk_name,
                description,
                schema,
                invoke,
                strict_json_schema=True,
                timeout_seconds=broker.timeout_seconds,
            )
        )
    return tools
