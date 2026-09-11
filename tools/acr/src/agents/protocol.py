"""Provider-neutral agent protocol.

No OpenAI SDK types cross this boundary. This keeps the core testable with a
fake backend and makes provider replacement a local adapter change.
"""

from __future__ import annotations

import asyncio
import uuid
from dataclasses import dataclass, field
from typing import Any, Mapping, Protocol

from pydantic import BaseModel

from ..core.invocations import AgentInput, AgentInstructions, AgentInvocation


@dataclass(frozen=True)
class ToolSpec:
    """Model-facing contract for a locally implemented tool.

    The SDK adapter exposes ``description`` and ``input_schema`` to the Agent
    and converts dots in ``name`` to underscores. ``capability`` is host-side
    policy metadata and is not shown to the model.
    """

    name: str
    description: str
    input_schema: dict[str, Any]
    capability: str = "read"


@dataclass
class AgentRequest:
    role: str
    instructions: str
    input_text: str
    output_schema: type[BaseModel] | None = None
    tools: tuple[ToolSpec, ...] = ()
    model: str = "gpt-5.5"
    max_turns: int | None = None
    timeout_seconds: float | None = None
    metadata: Mapping[str, str] = field(default_factory=dict)
    persona: str | None = None
    # New SDK path: stable instructions and volatile input are independently
    # addressable. The legacy fields remain accepted by fake/custom backends.
    invocation: AgentInvocation[BaseModel] | None = None

    @classmethod
    def from_invocation(cls, invocation: AgentInvocation[BaseModel]) -> "AgentRequest":
        return cls(
            role=invocation.role,
            instructions=invocation.instructions.text,
            input_text=invocation.input.content,
            output_schema=invocation.output_type,
            model=invocation.model,
            max_turns=invocation.run_limits.max_turns,
            timeout_seconds=invocation.run_limits.timeout_seconds,
            metadata=invocation.metadata,
            persona=invocation.persona,
            invocation=invocation,
        )

    @property
    def stable_instructions(self) -> AgentInstructions:
        if self.invocation is not None:
            return self.invocation.instructions
        return AgentInstructions((self.persona,) if self.persona else (), self.instructions, "legacy")

    @property
    def review_input(self) -> AgentInput:
        if self.invocation is not None:
            return self.invocation.input
        return AgentInput.from_text(self.input_text)


@dataclass
class AgentResponse:
    value: Any
    agent_run_id: str = field(default_factory=lambda: str(uuid.uuid4()))
    usage: dict[str, Any] = field(default_factory=dict)
    raw_output: str | None = None
    reasoning_summary: str | None = None


@dataclass(frozen=True)
class AgentEvent:
    event_type: str
    agent_run_id: str
    stage: str
    persona: str | None = None
    tool_name: str | None = None
    input_sha256: str | None = None
    output_sha256: str | None = None
    usage: Mapping[str, Any] = field(default_factory=dict)
    status: str = "completed"
    data: Mapping[str, Any] = field(default_factory=dict)


class CancellationToken:
    def __init__(self) -> None:
        self._event = asyncio.Event()

    def cancel(self) -> None:
        self._event.set()

    @property
    def cancelled(self) -> bool:
        return self._event.is_set()

    async def wait(self) -> None:
        await self._event.wait()

    def raise_if_cancelled(self) -> None:
        if self.cancelled:
            raise asyncio.CancelledError


class EventSink(Protocol):
    def __call__(self, event: AgentEvent) -> None: ...


class AgentBackend(Protocol):
    async def run(self, request: AgentRequest, *, event_sink: EventSink | None = None, cancellation: CancellationToken | None = None) -> AgentResponse: ...
