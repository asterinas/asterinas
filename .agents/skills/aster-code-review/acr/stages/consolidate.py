"""Apply consolidation's fix-only mapping without changing comment identity."""

from __future__ import annotations

import json

from ..agents.output_parser import parse_model
from ..agents.protocol import AgentBackend, AgentRequest
from ..agents.factories import AgentFactory
from ..config import RunConfig
from ..core.contracts import ConsolidationResult
from ..core.events import EventLogger
from ..agents.tracing import logger_sink
from .assemble import ReviewDocument
from .verify import comment_id


class ConsolidationError(RuntimeError):
    pass


async def consolidate_document(document: ReviewDocument, backend: AgentBackend, config: RunConfig, *, logger: EventLogger | None = None) -> ReviewDocument:
    if not document.comments:
        return document
    indexed = {comment_id(comment, index): comment.model_dump(exclude_none=True) for index, comment in enumerate(document.comments)}
    request = AgentRequest.from_invocation(AgentFactory(config).consolidation(json.dumps(indexed, ensure_ascii=False, sort_keys=True)))
    response = await backend.run(request, event_sink=logger_sink(logger, stage="consolidation") if logger is not None else None)
    if logger is not None:
        logger.agent_artifact(stage="consolidation", agent_run_id=response.agent_run_id, value=response.raw_output or response.value)
    result = parse_model(response.value, ConsolidationResult)
    known = set(indexed)
    # ``shared_fixes`` names clusters (for example ``shared-1``), while
    # ``comment_updates`` maps concrete comment IDs to either a cluster name
    # or an inline replacement.  Only the latter keys identify comments.
    unknown = set(result.comment_updates) - known
    if unknown:
        raise ConsolidationError(f"consolidation returned unknown comment IDs: {sorted(unknown)}")
    mappings = {
        cid: result.shared_fixes.get(value, value)
        for cid, value in result.comment_updates.items()
    }
    # Accept the original direct comment-id mapping form as well.
    mappings.update({cid: fix for cid, fix in result.shared_fixes.items() if cid in known})
    comments = [comment.model_copy(update={"fix": mappings[cid]}) if (cid := comment_id(comment, index)) in mappings else comment for index, comment in enumerate(document.comments)]
    return ReviewDocument(document.meta, comments, list(document.retracted), document.summary)
