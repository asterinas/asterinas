"""Locate the user's Claude CLI and redact inherited credentials."""

from __future__ import annotations

import os
import shutil
from typing import Any, Mapping

from .protocol import ClaudeAgentError


def find_cli() -> str:
    # The SDK prefers its bundled binary; explicitly select the executable
    # the user configured instead, without resolving symlinks or shell aliases.
    path = shutil.which("claude")
    if path is None:
        raise ClaudeAgentError(
            "Claude Code CLI was not found on PATH; install and configure claude "
            "in the same environment as ACR"
        )
    return path


def redact(value: Any) -> Any:
    """Keep inherited environment credentials out of adapter events."""
    if isinstance(value, str):
        for name in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"):
            secret = os.environ.get(name)
            if secret:
                value = value.replace(secret, "[REDACTED]")
        return value
    if isinstance(value, Mapping):
        return {key: redact(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [redact(item) for item in value]
    return value
