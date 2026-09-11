"""Versioned inputs passed from the control plane to agent backends.

The review input is intentionally separate from stable instructions.  This is
important for prompt caching and, more importantly, prevents one persona's
transcript or policy from becoming another persona's context.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Generic, Literal, Mapping, TypeVar

from pydantic import BaseModel

from .state import sha256_text

Persona = Literal["maintainability", "development", "security", "hardware", "documentation"]


@dataclass(frozen=True)
class AgentInstructions:
    personas: tuple[str, ...]
    text: str
    contract_version: str
    catalog_digests: Mapping[str, str] = field(default_factory=dict)
    sha256: str = ""

    def __post_init__(self) -> None:
        if not self.sha256:
            object.__setattr__(self, "sha256", sha256_text(self.text))


@dataclass(frozen=True)
class AgentInput:
    content: str
    sha256: str = ""

    def __post_init__(self) -> None:
        if not self.sha256:
            object.__setattr__(self, "sha256", sha256_text(self.content))

    @classmethod
    def from_text(cls, content: str) -> "AgentInput":
        return cls(content)


@dataclass(frozen=True)
class ToolPolicy:
    """Least-privilege policy attached to one invocation."""

    allow: tuple[str, ...] = ()
    network: Literal["disabled", "allowlisted"] = "disabled"
    writes: Literal["none", "workspace", "explicit"] = "none"
    max_output_bytes: int = 1_000_000
    timeout_seconds: float = 20.0


@dataclass(frozen=True)
class RunLimits:
    max_turns: int | None = None
    timeout_seconds: float | None = None
    retries: int = 0


T = TypeVar("T", bound=BaseModel)


@dataclass(frozen=True)
class AgentInvocation(Generic[T]):
    instructions: AgentInstructions
    input: AgentInput
    output_type: type[T]
    tool_policy: ToolPolicy = field(default_factory=ToolPolicy)
    run_limits: RunLimits = field(default_factory=RunLimits)
    role: str = "persona"
    persona: str | None = None
    model: str = "gpt-5.5"
    metadata: Mapping[str, str] = field(default_factory=dict)
