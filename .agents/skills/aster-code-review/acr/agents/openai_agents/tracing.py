"""Local OpenAI Agents SDK tracing for portable ACR runs.

The SDK invokes tracing processors synchronously.  This module therefore
keeps the processor small and append-only, while :class:`EventLogger` owns
redaction, truncation, permissions, and the GMT+8 timestamp format used by
the rest of ACR.
"""

from __future__ import annotations

import contextvars
import json
from contextlib import contextmanager
from typing import Any, Iterator

from ...config import RunConfig
from ...core.events import EventLogger


_LOCAL_TRACE_ACTIVE: contextvars.ContextVar[bool] = contextvars.ContextVar(
    "acr_local_trace_active", default=False
)


def local_trace_active() -> bool:
    """Return whether the current task is inside an ACR-managed SDK trace."""

    return _LOCAL_TRACE_ACTIVE.get()


def _jsonable(value: Any) -> Any:
    """Convert SDK/OpenAI objects to data that can safely enter JSONL."""

    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple, set, frozenset)):
        return [_jsonable(item) for item in value]
    model_dump = getattr(value, "model_dump", None)
    if callable(model_dump):
        try:
            return _jsonable(model_dump(mode="json"))
        except TypeError:
            return _jsonable(model_dump())
    export = getattr(value, "export", None)
    if callable(export):
        try:
            return _jsonable(export())
        except Exception:
            pass
    return str(value)


def _span_data(span: Any) -> dict[str, Any]:
    data = getattr(span, "span_data", None)
    exported = data.export() if data is not None and hasattr(data, "export") else data
    value = _jsonable(exported)
    result = value if isinstance(value, dict) else {"value": value}
    # In SDK 0.22 ResponseSpanData.export() deliberately emits only ID/usage.
    # Custom processors can still access the input and full Pydantic Response
    # held by the span, which are required for a complete local audit trail.
    if result.get("type") == "response" and data is not None:
        result["input"] = _jsonable(getattr(data, "input", None))
        result["response"] = _jsonable(getattr(data, "response", None))
    return result


def _span_context(span: Any, inherited: dict[str, str] | None = None) -> dict[str, str]:
    data = _span_data(span)
    context = dict(inherited or {})
    custom = data.get("data") if data.get("type") == "custom" else None
    if isinstance(custom, dict):
        for key in ("stage", "persona", "agent_run_id"):
            value = custom.get(f"acr_{key}")
            if isinstance(value, str) and value:
                context[key] = value
    return context


class LocalTraceProcessor:
    """Write SDK traces and spans to the run's local JSONL files."""

    def __init__(self, logger: EventLogger):
        self.logger = logger
        self._span_contexts: dict[str, dict[str, str]] = {}

    def on_trace_start(self, trace: Any) -> None:
        exported = _jsonable(trace.export()) or {}
        self.logger.sdk_trace(
            "sdk.trace.started",
            trace_id=getattr(trace, "trace_id", None),
            workflow_name=getattr(trace, "name", None),
            group_id=getattr(trace, "group_id", None),
            metadata=exported.get("metadata"),
        )

    def on_trace_end(self, trace: Any) -> None:
        exported = _jsonable(trace.export()) or {}
        self.logger.sdk_trace(
            "sdk.trace.completed",
            trace_id=getattr(trace, "trace_id", None),
            workflow_name=getattr(trace, "name", None),
            group_id=getattr(trace, "group_id", None),
            metadata=exported.get("metadata"),
        )

    def on_span_start(self, span: Any) -> None:
        parent_id = getattr(span, "parent_id", None)
        context = _span_context(span, self._span_contexts.get(parent_id))
        span_id = getattr(span, "span_id", None)
        if isinstance(span_id, str):
            self._span_contexts[span_id] = context
        self.logger.sdk_trace(
            "sdk.span.started",
            trace_id=getattr(span, "trace_id", None),
            span_id=span_id,
            parent_id=parent_id,
            started_at=getattr(span, "started_at", None),
            span_data=_span_data(span),
            **context,
        )

    def on_span_end(self, span: Any) -> None:
        span_id = getattr(span, "span_id", None)
        context = self._span_contexts.get(span_id, {})
        exported = _jsonable(span.export()) or {}
        if isinstance(exported, dict):
            exported["span_data"] = _span_data(span)
        self.logger.sdk_trace(
            "sdk.span.completed",
            trace_id=getattr(span, "trace_id", None),
            span_id=span_id,
            parent_id=getattr(span, "parent_id", None),
            started_at=getattr(span, "started_at", None),
            ended_at=getattr(span, "ended_at", None),
            span=exported,
            **context,
        )
        if isinstance(span_id, str):
            self._span_contexts.pop(span_id, None)

    def shutdown(self) -> None:
        self._span_contexts.clear()

    def force_flush(self) -> None:
        return None


@contextmanager
def sdk_trace_context(
    logger: EventLogger,
    config: RunConfig,
    *,
    workflow_name: str = "ACR review workflow",
    metadata: dict[str, Any] | None = None,
) -> Iterator[None]:
    """Install one local processor and one workflow trace for an ACR run.

    The SDK has global processor configuration.  ACR replaces it for the
    duration of a run and clears it afterward, preventing the default OpenAI
    exporter from receiving relay prompts or tool content.
    """

    if not config.tracing_enabled:
        yield
        return
    try:
        from agents import flush_traces, set_trace_processors, set_tracing_disabled, trace
    except ImportError:
        # The fake backend and dependency-free installs remain usable.
        yield
        return

    processor = LocalTraceProcessor(logger)
    token = _LOCAL_TRACE_ACTIVE.set(True)
    set_trace_processors([processor])
    set_tracing_disabled(False)
    trace_metadata = {"acr_run_id": logger.run_id, **(metadata or {})}
    try:
        with trace(workflow_name, metadata=trace_metadata):
            yield
    finally:
        try:
            flush_traces()
        finally:
            processor.shutdown()
            set_trace_processors([])
            set_tracing_disabled(True)
            _LOCAL_TRACE_ACTIVE.reset(token)


def sdk_trace_json(value: Any) -> str:
    """Serialize an SDK value for diagnostics without leaking object reprs."""

    return json.dumps(_jsonable(value), ensure_ascii=False, default=str)
