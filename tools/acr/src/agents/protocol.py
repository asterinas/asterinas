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

from ..core.invocations import AgentInput, AgentInstructions, AgentInvocation, RunLimits


@dataclass(frozen=True)
class ToolSpec:
    """Model-facing contract for a locally implemented tool.

    The SDK adapter exposes `description` and `input_schema` to the Agent
    and converts dots in `name` to underscores. `capability` is host-side
    policy metadata and is not shown to the model.
    """

    name: str
    description: str
    input_schema: dict[str, Any]
    capability: str = "read"


class AgentRequest:
    """Backend-facing view of a single invocation."""

    def __init__(
        self,
        role: str | None = None,
        instructions: str | None = None,
        input_text: str | None = None,
        output_schema: type[BaseModel] | None = None,
        tools: tuple[ToolSpec, ...] = (),
        model: str = "gpt-5.5",
        timeout_seconds: float | None = None,
        metadata: Mapping[str, str] | None = None,
        persona: str | None = None,
        invocation: AgentInvocation[BaseModel] | None = None,
    ) -> None:
        self._legacy = invocation is None
        if invocation is None:
            if role is None or instructions is None or input_text is None:
                raise TypeError("provide an invocation or role, instructions, and input_text")
            invocation = AgentInvocation(
                instructions=AgentInstructions((persona,) if persona else (), instructions, "legacy"),
                input=AgentInput.from_text(input_text),
                output_type=output_schema,
                run_limits=RunLimits(timeout_seconds=timeout_seconds),
                role=role,
                persona=persona,
                model=model,
                metadata=metadata if metadata is not None else {},
            )
        self._invocation = invocation
        self.tools = tools

    @classmethod
    def from_invocation(cls, invocation: AgentInvocation[BaseModel]) -> "AgentRequest":
        return cls(invocation=invocation)

    @property
    def invocation(self) -> AgentInvocation[BaseModel] | None:
        """The explicit invocation, if supplied, for backend tool policy."""
        return None if self._legacy else self._invocation

    @property
    def stable_instructions(self) -> AgentInstructions:
        return self._invocation.instructions

    @property
    def review_input(self) -> AgentInput:
        return self._invocation.input

    @property
    def role(self) -> str:
        return self._invocation.role

    @property
    def instructions(self) -> str:
        return self.stable_instructions.text

    @property
    def input_text(self) -> str:
        return self.review_input.content

    @property
    def output_schema(self) -> type[BaseModel] | None:
        return self._invocation.output_type

    @property
    def model(self) -> str:
        return self._invocation.model

    @property
    def timeout_seconds(self) -> float | None:
        return self._invocation.run_limits.timeout_seconds

    @property
    def metadata(self) -> Mapping[str, str]:
        return self._invocation.metadata

    @property
    def persona(self) -> str | None:
        return self._invocation.persona


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
