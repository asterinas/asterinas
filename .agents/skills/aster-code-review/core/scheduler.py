"""Concurrent persona scheduling with explicit fan-out/combined semantics."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Mapping

from ..agents.output_parser import parse_comments
from ..agents.protocol import AgentBackend, AgentEvent, AgentRequest, CancellationToken
from .invocations import AgentInvocation
from ..config import RunConfig
from .contracts import CommentsEnvelope
from .events import EventLogger


class SchedulerError(RuntimeError):
    pass


@dataclass(frozen=True)
class PersonaPass:
    persona: str
    prompt: str


class PassScheduler:
    def __init__(self, backend: AgentBackend, config: RunConfig, logger: EventLogger | None = None):
        self.backend = backend
        self.config = config
        self.logger = logger

    async def run(self, prompts: Mapping[str, str | AgentInvocation], personas: tuple[str, ...]) -> dict[str, list[dict]]:
        if not personas:
            return {}
        if self.config.fan_out:
            values = await self._fan_out(prompts, personas)
        else:
            values = await self._combined(prompts, personas)
        return values

    async def _fan_out(self, prompts: Mapping[str, str | AgentInvocation], personas: tuple[str, ...]) -> dict[str, list[dict]]:
        semaphore = asyncio.Semaphore(self.config.max_concurrency)

        async def one(persona: str) -> tuple[str, list[dict]]:
            if persona not in prompts:
                raise SchedulerError(f"missing prompt for {persona}")
            async with semaphore:
                value = prompts[persona]
                request = AgentRequest.from_invocation(value) if isinstance(value, AgentInvocation) else AgentRequest(
                    role="persona", instructions=f"Review only the {persona} persona scope and return the pass-contract JSON array.",
                    input_text=value, output_schema=CommentsEnvelope, model=self.config.model,
                    max_turns=self.config.max_turns, timeout_seconds=self.config.timeout_seconds, persona=persona,
                )
                response = await self._call(request, stage="persona", persona=persona)
                if self.logger is not None:
                    self.logger.agent_artifact(stage="persona", persona=persona, agent_run_id=response.agent_run_id, value=response.raw_output or response.value)
                comments = parse_comments(response.value)
                return persona, [comment.model_dump(exclude_none=True) for comment in comments]

        results = await asyncio.gather(*(one(persona) for persona in personas), return_exceptions=True)
        errors = [result for result in results if isinstance(result, Exception)]
        if errors:
            raise SchedulerError("; ".join(str(error) for error in errors))
        return dict(results)  # type: ignore[arg-type]

    async def _combined(self, prompts: Mapping[str, str | AgentInvocation], personas: tuple[str, ...]) -> dict[str, list[dict]]:
        missing = [persona for persona in personas if persona not in prompts]
        if missing:
            raise SchedulerError(f"missing prompt(s): {', '.join(missing)}")
        first = prompts[personas[0]]
        if isinstance(first, AgentInvocation):
            # Combined mode receives one invocation with all persona blocks.
            request = AgentRequest.from_invocation(first)
        else:
            marker = "===== REVIEW INPUT ====="
            stable_blocks = [str(prompts[persona]).split(marker, 1)[0].rstrip() for persona in personas]
            review_input = str(first).split(marker, 1)[1] if marker in str(first) else ""
            combined = "\n\n".join(stable_blocks) + "\n" + marker + review_input
            request = AgentRequest(role="persona", instructions="Review every included persona scope and set each comment's persona field correctly.", input_text=combined, output_schema=CommentsEnvelope, model=self.config.model, max_turns=self.config.max_turns, timeout_seconds=self.config.timeout_seconds)
        response = await self._call(request, stage="persona", persona=None)
        if self.logger is not None:
            self.logger.agent_artifact(stage="persona", agent_run_id=response.agent_run_id, value=response.raw_output or response.value)
        comments = parse_comments(response.value)
        result = {persona: [] for persona in personas}
        for comment in comments:
            if comment.persona not in result:
                raise SchedulerError(f"combined pass returned inactive persona {comment.persona}")
            result[comment.persona].append(comment.model_dump(exclude_none=True))
        return result

    async def _call(self, request: AgentRequest, *, stage: str, persona: str | None):
        cancellation = CancellationToken()

        def sink(event: AgentEvent) -> None:
            if self.logger is not None:
                self.logger.agent(event.event_type, stage=stage, persona=persona, agent_run_id=event.agent_run_id, tool_name=event.tool_name, input_sha256=event.input_sha256, output_sha256=event.output_sha256, usage=dict(event.usage), status=event.status, **dict(event.data))

        error: Exception | None = None
        for attempt in range(self.config.retries + 1):
            try:
                return await self.backend.run(request, event_sink=sink, cancellation=cancellation)
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                error = exc
                if attempt < self.config.retries:
                    await asyncio.sleep(min(2.0, 0.1 * (2**attempt)))
        assert error is not None
        raise error
