"""Verification agent plus deterministic verdict application."""

from __future__ import annotations

import hashlib
import json
from typing import Any

from ..agents.output_parser import parse_model
from ..agents.protocol import AgentBackend, AgentRequest
from ..agents.factories import AgentFactory
from ..config import RunConfig
from ..core.contracts import ReviewComment, VerificationEnvelope
from ..core.events import EventLogger
from ..agents.tracing import logger_sink
from .assemble import ReviewDocument


class VerificationError(RuntimeError):
    code = "VERIFICATION_ERROR"


def comment_id(comment: ReviewComment, index: int) -> str:
    payload = json.dumps(comment.model_dump(exclude_none=True), sort_keys=True, ensure_ascii=False)
    return f"c{index + 1}-{hashlib.sha256(payload.encode()).hexdigest()[:12]}"


async def verify_document(document: ReviewDocument, backend: AgentBackend, config: RunConfig, *, logger: EventLogger | None = None) -> ReviewDocument:
    if not document.comments:
        return document
    ids = {comment_id(comment, index): comment for index, comment in enumerate(document.comments)}
    payload = json.dumps({cid: comment.model_dump(exclude_none=True) for cid, comment in ids.items()}, ensure_ascii=False, sort_keys=True)
    request = AgentRequest.from_invocation(AgentFactory(config).verification(payload))
    response = await backend.run(request, event_sink=logger_sink(logger, stage="verification") if logger is not None else None)
    if logger is not None:
        logger.agent_artifact(stage="verification", agent_run_id=response.agent_run_id, value=response.raw_output or response.value)
    result = parse_model(response.value, VerificationEnvelope)
    by_id = {item.comment_id: item for item in result.items}
    missing = set(ids) - set(by_id)
    unknown = set(by_id) - set(ids)
    if missing or unknown:
        raise VerificationError(f"verification results must cover comments exactly; missing={sorted(missing)}, unknown={sorted(unknown)}")
    kept: list[ReviewComment] = []
    retracted = list(document.retracted)
    for index, comment in enumerate(document.comments):
        cid = comment_id(comment, index)
        item = by_id[cid]
        if item.verdict == "refuted":
            retracted.append((cid, item.reason))
        elif item.verdict == "uncertain":
            if not comment.problem.startswith("(unverified) "):
                comment = comment.model_copy(update={"problem": "(unverified) " + comment.problem})
            kept.append(comment)
        else:
            kept.append(comment)
    verified = ReviewDocument(document.meta, kept, retracted, document.summary)
    return verified
