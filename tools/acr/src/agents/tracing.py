"""Map provider-neutral agent events into the ACR event logger."""

from __future__ import annotations

from ..core.events import EventLogger
from .protocol import AgentEvent


def logger_sink(logger: EventLogger, *, stage: str, persona: str | None = None):
    def sink(event: AgentEvent) -> None:
        logger.agent(event.event_type, stage=stage, persona=persona, agent_run_id=event.agent_run_id, tool_name=event.tool_name, input_sha256=event.input_sha256, output_sha256=event.output_sha256, usage=dict(event.usage), status=event.status, **dict(event.data))
    return sink
