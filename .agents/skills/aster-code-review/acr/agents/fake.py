"""Deterministic backend used by model-free tests and initial benchmarks."""

from __future__ import annotations

import asyncio
import json
import re
from collections.abc import Callable
from typing import Any

from .protocol import AgentBackend, AgentEvent, AgentRequest, AgentResponse, CancellationToken
from ..core.contracts import CommentsEnvelope, ConsolidationResult, SummaryResult, VerificationEnvelope


class FakeBackend:
    """A no-network backend.

    ``responses`` can provide role/persona-specific values for integration
    tests. With no mapping, persona passes return an empty fragment and the
    refinement agents preserve all comments.
    """

    def __init__(self, responses: dict[str, Any] | None = None, *, delay: float = 0.0):
        self.responses = responses or {}
        self.delay = delay
        self.requests: list[AgentRequest] = []

    async def run(self, request: AgentRequest, *, event_sink: Callable[[AgentEvent], None] | None = None, cancellation: CancellationToken | None = None) -> AgentResponse:
        self.requests.append(request)
        token = cancellation or CancellationToken()
        token.raise_if_cancelled()
        if self.delay:
            await asyncio.sleep(self.delay)
        token.raise_if_cancelled()
        key = request.persona or request.role
        value = self.responses.get(key)
        if value is None:
            if request.role == "persona":
                value = []
            elif request.role == "verification":
                try:
                    comments = json.loads(request.input_text)
                    value = {
                        "items": [
                            {
                                "comment_id": comment_id,
                                "verdict": "confirmed",
                                "premise": "No external premise was disproved by the fake backend.",
                                "evidence": [],
                                "reason": "Fake backend preserves the comment for model-free testing.",
                            }
                            for comment_id in comments
                        ]
                    }
                except (TypeError, json.JSONDecodeError):
                    value = {"items": []}
            elif request.role == "consolidation":
                value = {"shared_fixes": {}, "comment_updates": {}}
            elif request.role == "summary":
                value = {"summary": "No review findings were produced."}
            elif request.role == "grader":
                try:
                    expected = json.loads(request.input_text)["expected"]
                    defect_ids = [
                        int(match.group(1))
                        for line in expected.splitlines()
                        if (match := re.match(r"^([1-9][0-9]*)\.\s+location:", line))
                    ]
                    value = {
                        "results": [
                            {
                                "defect": defect_id,
                                "status": "miss",
                                "reason": "The fake backend does not produce matching findings.",
                            }
                            for defect_id in defect_ids
                        ]
                    }
                except (KeyError, TypeError, json.JSONDecodeError):
                    value = {"results": []}
            else:
                value = {}
        if callable(value):
            value = value(request)
        if isinstance(value, str):
            raw = value
        else:
            raw = json.dumps(value, ensure_ascii=False, default=str)
        response = AgentResponse(value=value, raw_output=raw)
        if event_sink:
            event_sink(AgentEvent(event_type="completed", agent_run_id=response.agent_run_id, stage=request.role, persona=request.persona, output_sha256=None))
        return response
