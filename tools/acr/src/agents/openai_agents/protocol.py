"""Adapt OpenAI results and provider errors at the shared ACR boundary."""

from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any


class OpenAIAgentError(RuntimeError):
    """An SDK/provider failure that should fail the current stage."""


def _http_error_type(error: Exception) -> str | None:
    current: Exception | None = error
    seen: set[int] = set()
    while current is not None and id(current) not in seen:
        seen.add(id(current))
        body = getattr(current, "body", None)
        if isinstance(body, Mapping):
            payload = body.get("error", body)
            if isinstance(payload, Mapping) and isinstance(payload.get("type"), str):
                return payload["type"]
        next_error = current.__cause__ or current.__context__
        current = next_error if isinstance(next_error, Exception) else None
    return None


def _dump(value: Any) -> str:
    if hasattr(value, "model_dump_json"):
        return value.model_dump_json()
    try:
        return json.dumps(value, ensure_ascii=False, default=str)
    except TypeError:
        return str(value)


def _usage(result: Any) -> dict[str, Any]:
    usage = getattr(result, "context_wrapper", None)
    usage = getattr(usage, "usage", None) or getattr(result, "usage", None)
    if usage is None:
        return {"reported": False}
    if hasattr(usage, "model_dump"):
        value = usage.model_dump()
    elif hasattr(usage, "__dict__"):
        value = dict(usage.__dict__)
    else:
        value = {"value": str(usage)}
    details = value.get("input_tokens_details")
    if hasattr(details, "model_dump"):
        details = details.model_dump()
    if not isinstance(details, Mapping):
        details = {}
    value.update(
        input=value.get("input_tokens"),
        output=value.get("output_tokens"),
        total=value.get("total_tokens"),
        cache_read=details.get("cached_tokens"),
        cache_write=details.get("cache_write_tokens"),
        turns=value.get("requests"),
        tool_calls=sum(
            getattr(item, "type", None) == "tool_call_item"
            for item in getattr(result, "new_items", ()) or ()
        ),
        reported=True,
    )
    # SDK versions have returned nested RequestUsage objects from model_dump;
    # event logs are JSONL and must never fail after a successful API call.
    try:
        return json.loads(json.dumps(value, ensure_ascii=False, default=str))
    except (TypeError, ValueError):
        return {"value": str(value)}


def _reasoning_summary(result: Any) -> str | None:
    for item in getattr(result, "new_items", ()) or ():
        raw = getattr(item, "raw_item", None)
        summary = getattr(raw, "summary", None)
        if summary:
            return str(summary)
    return None
