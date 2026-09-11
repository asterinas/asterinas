"""Central backend construction and optional lifecycle cleanup."""

from __future__ import annotations

import inspect
from pathlib import Path

from ..config import RunConfig
from ..tools.builtin import default_provider
from ..tools.protocol import ToolBroker
from .fake import FakeBackend
from .openai_agents import OpenAIAgentsBackend
from .protocol import AgentBackend


def create_backend(
    config: RunConfig,
    *,
    repo_root: Path,
    run_root: Path,
) -> AgentBackend:
    if config.backend == "fake":
        return FakeBackend()
    broker = ToolBroker(default_provider(repo_root)) if config.tools_enabled else None
    if config.backend == "openai-agents":
        return OpenAIAgentsBackend(config, broker=broker)
    if config.backend == "pi-agent":
        from .pi import PiAgentBackend

        return PiAgentBackend(
            config,
            repo_root=repo_root,
            run_root=run_root,
            broker=broker,
        )
    raise ValueError(f"unsupported backend: {config.backend}")


async def close_backend(backend: AgentBackend | None) -> None:
    if backend is None:
        return
    close = getattr(backend, "aclose", None)
    if close is None:
        return
    result = close()
    if inspect.isawaitable(result):
        await result
