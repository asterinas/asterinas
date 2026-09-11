"""Summary agent; only its isolated summary text reaches the Markdown sink."""

from __future__ import annotations

import json

from ..agents.output_parser import parse_model
from ..agents.protocol import AgentBackend, AgentRequest
from ..agents.factories import AgentFactory
from ..config import RunConfig
from ..core.contracts import SummaryResult
from ..core.events import EventLogger
from ..agents.tracing import logger_sink
from .assemble import ReviewDocument, render_markdown


class SummaryError(RuntimeError):
    pass


async def summarize_document(document: ReviewDocument, backend: AgentBackend, config: RunConfig, *, logger: EventLogger | None = None) -> ReviewDocument:
    payload = json.dumps(document.as_json(), ensure_ascii=False, sort_keys=True)
    request = AgentRequest.from_invocation(AgentFactory(config).summary(payload))
    response = await backend.run(request, event_sink=logger_sink(logger, stage="summary") if logger is not None else None)
    if logger is not None:
        logger.agent_artifact(stage="summary", agent_run_id=response.agent_run_id, value=response.raw_output or response.value)
    result = parse_model(response.value, SummaryResult)
    return ReviewDocument(document.meta, list(document.comments), list(document.retracted), result.summary)
