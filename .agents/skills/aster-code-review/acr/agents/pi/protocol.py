"""Versioned DTO helpers for the ACR/Pi JSONL bridge."""

from __future__ import annotations

from typing import Any, Mapping


PROTOCOL_VERSION = 1
REQUIRED_CAPABILITIES = frozenset(
    {
        "structured_delegation",
        "runtime_agent_registration",
        "usage",
        "tool_proxy",
        "cancellation",
    }
)


class PiProtocolError(RuntimeError):
    pass


def validate_ready(frame: Mapping[str, Any]) -> None:
    if frame.get("version") != PROTOCOL_VERSION or frame.get("type") != "ready":
        raise PiProtocolError("Pi bridge did not send a compatible ready frame")
    capabilities = frame.get("capabilities")
    if not isinstance(capabilities, list):
        raise PiProtocolError("Pi bridge ready frame has no capability list")
    missing = REQUIRED_CAPABILITIES - set(capabilities)
    if missing:
        raise PiProtocolError(
            "Pi bridge lacks required capabilities: " + ", ".join(sorted(missing))
        )


def normalize_usage(value: object) -> dict[str, Any]:
    if not isinstance(value, Mapping):
        return {"reported": False}
    aliases = {
        "input": "input",
        "output": "output",
        "cache_read": "cache_read",
        "cache_write": "cache_write",
        "total": "total",
        "cost": "cost",
        "turns": "turns",
        "tool_calls": "tool_calls",
        "duration_ms": "duration_ms",
    }
    normalized = {target: value.get(source) for source, target in aliases.items()}
    normalized["reported"] = any(item is not None for item in normalized.values())
    return normalized
