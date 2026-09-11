"""Normalize Claude results at the provider-neutral ACR boundary."""

from __future__ import annotations

import math
from typing import Any, Mapping

from pydantic import ValidationError


class ClaudeAgentError(RuntimeError):
    """A Claude run failed; no review result may be inferred from the failure."""


def normalize_usage(result: Any, *, tool_calls: int) -> dict[str, Any]:
    raw = result.usage if isinstance(result.usage, Mapping) else {}

    def count(name: str) -> int | None:
        value = raw.get(name)
        return value if type(value) is int and value >= 0 else None

    tokens = {
        "input": count("input_tokens"),
        "output": count("output_tokens"),
        "cache_read": count("cache_read_input_tokens"),
        "cache_write": count("cache_creation_input_tokens"),
    }
    values = tuple(tokens.values())
    cost = result.total_cost_usd
    if type(cost) not in (int, float) or not math.isfinite(cost) or cost < 0:
        cost = None
    return {
        **tokens,
        "total": sum(values) if all(value is not None for value in values) else None,
        "cost": cost,
        "turns": result.num_turns,
        "tool_calls": tool_calls,
        "duration_ms": result.duration_api_ms,
        "reported": any(value is not None for value in values) or cost is not None,
    }


def validate_result(result: Any, output_schema: Any) -> dict[str, Any]:
    if result.subtype != "success" or result.is_error:
        detail = "; ".join(result.errors or []) or result.result or result.subtype
        raise ClaudeAgentError(f"Claude agent failed: {detail}")
    if result.structured_output is None:
        raise ClaudeAgentError("Claude agent returned no structured output")
    try:
        return output_schema.model_validate(result.structured_output).model_dump(mode="json")
    except ValidationError as exc:
        # ValidationError.__str__ includes input values even when transcript
        # logging is disabled. Keep useful field/type diagnostics without them.
        errors = exc.errors(include_input=False, include_context=False, include_url=False)
        raise ClaudeAgentError(f"invalid Claude structured output: {errors}") from exc
