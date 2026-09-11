"""MCP registration boundary.

MCP transports are intentionally not started by the initial implementation;
servers must first be registered with the broker and an explicit agent allowlist.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class McpServerConfig:
    name: str
    transport: str
    command: tuple[str, ...] = ()
    url: str | None = None
    allow_for: tuple[str, ...] = ()
    tools: tuple[str, ...] = ()
    timeout_seconds: float = 20.0

    def validate(self) -> "McpServerConfig":
        if self.transport not in {"stdio", "sse", "http", "streamable-http"}:
            raise ValueError(f"unsupported MCP transport: {self.transport}")
        if self.transport == "stdio" and not self.command:
            raise ValueError("stdio MCP server requires a command")
        if self.transport != "stdio" and not self.url:
            raise ValueError("HTTP MCP server requires a URL")
        return self


class McpToolProvider:
    """Placeholder provider; SDK-specific MCP clients stay out of core."""

    def __init__(self, servers: tuple[McpServerConfig, ...] = ()):
        self.servers = tuple(server.validate() for server in servers)

    def specs(self):
        return ()

    async def invoke(self, name: str, arguments: dict[str, Any]):
        raise RuntimeError(f"MCP tool is not configured: {name}")
