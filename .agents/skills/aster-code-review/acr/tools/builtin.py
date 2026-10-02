"""Default read-only tool registry for local repository review."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..agents.protocol import ToolSpec
from .git import GitToolProvider
from .guidelines import GuidelineToolProvider
from .protocol import ToolProvider, ToolResult
from .repository import SourceToolProvider


_REGISTERED_PROVIDERS: list[ToolProvider] = []


def register_tool_provider(provider: ToolProvider) -> None:
    """Register an explicitly trusted provider for all backend adapters."""

    existing = {
        spec.name
        for registered in _REGISTERED_PROVIDERS
        for spec in registered.specs()
    }
    names = [spec.name for spec in provider.specs()]
    if len(names) != len(set(names)):
        raise ValueError("custom tool provider contains duplicate tool names")
    collisions = existing & set(names)
    if collisions:
        raise ValueError("custom tool names already registered: " + ", ".join(sorted(collisions)))
    _REGISTERED_PROVIDERS.append(provider)


class CompositeToolProvider:
    def __init__(self, *providers: ToolProvider):
        self.providers = tuple(providers)
        self._owners: dict[str, ToolProvider] = {}
        for provider in self.providers:
            for spec in provider.specs():
                if spec.name in self._owners:
                    raise ValueError(f"duplicate tool name: {spec.name}")
                self._owners[spec.name] = provider

    def specs(self) -> tuple[ToolSpec, ...]:
        return tuple(spec for provider in self.providers for spec in provider.specs())

    async def invoke(self, name: str, arguments: dict[str, Any]) -> ToolResult:
        provider = self._owners.get(name)
        if provider is None:
            return ToolResult(False, error_code="TOOL_DENIED")
        return await provider.invoke(name, arguments)


def default_provider(repo_root: Path) -> CompositeToolProvider:
    return CompositeToolProvider(
        SourceToolProvider(repo_root),
        GitToolProvider(repo_root),
        GuidelineToolProvider(repo_root),
        *_REGISTERED_PROVIDERS,
    )
