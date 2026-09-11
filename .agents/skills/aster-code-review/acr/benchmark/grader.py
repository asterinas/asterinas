"""Separate benchmark grader agent protocol."""

from __future__ import annotations

import json
import re
from collections import Counter
from contextlib import nullcontext
from dataclasses import dataclass

from ..agents.output_parser import parse_model
from ..agents.protocol import AgentBackend, AgentRequest
from ..agents.factories import AgentFactory
from ..config import RunConfig
from ..core.contracts import GraderEnvelope, GraderItem
from ..core.events import EventLogger
from ..agents.tracing import logger_sink
from ..agents.openai_agents import sdk_trace_context


EXPECTED_ID_RE = re.compile(r"^(?P<id>[1-9][0-9]*)\.\s+location:")


class GraderError(RuntimeError):
    pass


@dataclass(frozen=True)
class RecallReport:
    expected: int
    caught: int
    partial: int
    miss: int
    not_caught: tuple[GraderItem, ...]

    @property
    def recall(self) -> float:
        return self.caught / self.expected

    def as_dict(self) -> dict[str, object]:
        return {
            "recall": self.recall,
            "expected": self.expected,
            "caught": self.caught,
            "partial": self.partial,
            "miss": self.miss,
            "not_caught": [item.model_dump() for item in self.not_caught],
        }


def calculate_recall(expected: str, result: GraderEnvelope) -> RecallReport:
    """Validate exact grader coverage and compute strict expected-defect recall."""

    expected_ids = [
        int(match.group("id"))
        for line in expected.splitlines()
        if (match := EXPECTED_ID_RE.match(line))
    ]
    if not expected_ids or expected_ids != list(range(1, len(expected_ids) + 1)):
        raise GraderError("expected defects must be numbered contiguously")

    by_id: dict[int, GraderItem] = {}
    for item in result.results:
        if item.defect in by_id:
            raise GraderError(f"grader returned duplicate defect ID: {item.defect}")
        by_id[item.defect] = item

    missing = set(expected_ids) - set(by_id)
    unknown = set(by_id) - set(expected_ids)
    if missing or unknown:
        raise GraderError(
            "grader results must cover expected defects exactly; "
            f"missing={sorted(missing)}, unknown={sorted(unknown)}"
        )

    ordered = tuple(by_id[defect_id] for defect_id in expected_ids)
    counts = Counter(item.status for item in ordered)
    return RecallReport(
        expected=len(expected_ids),
        caught=counts["caught"],
        partial=counts["partial"],
        miss=counts["miss"],
        not_caught=tuple(item for item in ordered if item.status != "caught"),
    )


async def grade(expected: str, produced_review: str, backend: AgentBackend, config: RunConfig, *, logger: EventLogger | None = None) -> GraderEnvelope:
    request = AgentRequest.from_invocation(AgentFactory(config).grader(json.dumps({"expected": expected, "produced_review": produced_review}, ensure_ascii=False)))
    trace_scope = (
        sdk_trace_context(logger, config, workflow_name="ACR benchmark grader")
        if logger is not None
        else nullcontext()
    )
    with trace_scope:
        response = await backend.run(request, event_sink=logger_sink(logger, stage="grader") if logger is not None else None)
        if logger is not None:
            logger.agent_artifact(stage="grader", agent_run_id=response.agent_run_id, value=response.raw_output or response.value)
            logger.sdk_trace(
                "workflow.final_output",
                stage="grader",
                agent_run_id=response.agent_run_id,
                output=response.raw_output or response.value,
            )
    result = parse_model(response.value, GraderEnvelope)
    calculate_recall(expected, result)
    return result
