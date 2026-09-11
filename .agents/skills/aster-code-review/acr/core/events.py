"""Structured, redacted event logging for runs and agent calls."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import time
import uuid
from datetime import datetime
from pathlib import Path
from typing import Any, Mapping

from .state import GMT8


_SECRET_KEY = re.compile(
    r"api[_-]?key|authorization|(?:access|refresh|auth|bearer)[_-]?token|"
    r"client[_-]?secret|password|credentials?|(?:^|[_-])token$|(?:^|[_-])secret$",
    re.IGNORECASE,
)


def _redact(value: Any, *, max_string_length: int = 4096) -> Any:
    if isinstance(value, Mapping):
        return {
            str(k): "[REDACTED]"
            if _SECRET_KEY.search(str(k))
            else _redact(v, max_string_length=max_string_length)
            for k, v in value.items()
        }
    if isinstance(value, list):
        return [_redact(item, max_string_length=max_string_length) for item in value]
    if isinstance(value, tuple):
        return [_redact(item, max_string_length=max_string_length) for item in value]
    if isinstance(value, str):
        # Replace values from all credential-like environment variables, not
        # only the conventional OpenAI names (portable configs may choose a
        # provider-specific ``api_key_env``).
        for key, secret in os.environ.items():
            if secret and (_SECRET_KEY.search(key) or key in {"OPENAI_BASE_URL", "OPENAI_API_BASE"}):
                value = value.replace(secret, "[REDACTED]")
        if len(value) > max_string_length:
            return f"[omitted sha256={hashlib.sha256(value.encode()).hexdigest()} length={len(value)}]"
    return value


class EventLogger:
    """Write a main JSONL stream and isolated per-agent streams.

    Main events are also printed in a compact form. Hidden chain-of-thought is
    never inferred; callers can explicitly record only provider-published
    reasoning summaries.
    """

    def __init__(self, root: Path, run_id: str, *, benchmark_id: str | None = None, max_content_bytes: int = 65536):
        self.root = root
        self.run_id = run_id
        self.benchmark_id = benchmark_id
        self.max_content_bytes = max(256, int(max_content_bytes))
        self._startup_duration_ms: dict[str, int] = {}
        self._usage_entries: dict[str, dict[str, Any]] = {}
        self.main_path = root / "main.jsonl"
        self.sdk_trace_path = root / "sdk-trace.jsonl"
        self.main_path.touch(mode=0o600, exist_ok=True)
        self.sdk_trace_path.touch(mode=0o600, exist_ok=True)

    def emit(self, event_type: str, *, stage: str | None = None, persona: str | None = None, agent_run_id: str | None = None, **data: Any) -> str:
        event_id = str(uuid.uuid4())
        event = {
            "timestamp": datetime.now(GMT8).isoformat(),
            "run_id": self.run_id,
            "benchmark_id": self.benchmark_id,
            "event_id": event_id,
            "event_type": event_type,
            "stage": stage,
            "persona": persona,
            "agent_run_id": agent_run_id,
            **_redact(data, max_string_length=self.max_content_bytes),
        }
        self._append(self.main_path, event)
        # Tool calls are fully represented in JSONL. Printing each one made a
        # real review emit hundreds of indistinguishable ``persona`` lines.
        if not event_type.startswith(("tool.", "pi.", "agent.reply")):
            label = f"{stage or event_type}"
            if event_type in {"started", "completed", "error", "cancelled"}:
                label += f" {event_type}"
            elif event_type == "run.completed":
                label += " completed"
            if persona:
                label += f" persona={persona}"
            print(f"[acr] {label}", file=sys.stderr, flush=True)
        return event_id

    def agent(self, event_type: str, *, stage: str, persona: str | None, agent_run_id: str, **data: Any) -> str:
        event_id = self.emit(event_type, stage=stage, persona=persona, agent_run_id=agent_run_id, **data)
        scope = Path("personas") / persona if persona else Path(stage)
        path = self.root / "agents" / scope / "events.jsonl"
        path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        event = {
            "timestamp": datetime.now(GMT8).isoformat(),
            "run_id": self.run_id,
            "benchmark_id": self.benchmark_id,
            "event_id": event_id,
            "agent_run_id": agent_run_id,
            "stage": stage,
            "role": stage,
            "persona": persona,
            "event_type": event_type,
            **_redact(data, max_string_length=self.max_content_bytes),
        }
        self._append(path, event)
        transcript_path = self.root / "agents" / scope / "transcript.jsonl"
        if event_type.startswith(("tool.", "pi.", "agent.reply")):
            self._append(transcript_path, event)
        if event_type == "pi.ready":
            startup = data.get("startup_duration_ms")
            if isinstance(startup, int):
                self._startup_duration_ms[agent_run_id] = startup
        if event_type == "completed" and isinstance(data.get("usage"), Mapping):
            self._record_usage(
                scope=scope,
                stage=stage,
                persona=persona,
                agent_run_id=agent_run_id,
                usage=data["usage"],
                total_duration_ms=data.get("duration_ms"),
                model=data.get("model"),
            )
        return event_id

    def agent_artifact(self, *, stage: str, agent_run_id: str, value: Any, persona: str | None = None, kind: str = "response") -> Path:
        scope = Path("personas") / persona if persona else Path(stage)
        path = self.root / "agents" / scope / f"{kind}-{agent_run_id}.json"
        path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        with path.open("w", encoding="utf-8") as stream:
            if isinstance(value, str):
                stream.write(str(_redact(value, max_string_length=self.max_content_bytes)))
            else:
                json.dump(_redact(value, max_string_length=self.max_content_bytes), stream, ensure_ascii=False, indent=2, default=str)
                stream.write("\n")
        path.chmod(0o600)
        return path

    def sdk_trace(self, event_type: str, *, stage: str | None = None, persona: str | None = None,
                  agent_run_id: str | None = None, **data: Any) -> str:
        """Persist one OpenAI Agents SDK trace/ span event.

        SDK processors are synchronous callbacks, so this method deliberately
        performs a small append-only write and never prints model transcripts
        to the terminal.  The same event is available in ``main.jsonl`` and
        the dedicated ``sdk-trace.jsonl`` stream; scoped agent copies make it
        possible to inspect one persona without filtering the whole run.
        """
        event_id = str(uuid.uuid4())
        event = {
            "timestamp": datetime.now(GMT8).isoformat(),
            "run_id": self.run_id,
            "benchmark_id": self.benchmark_id,
            "event_id": event_id,
            "event_type": event_type,
            "stage": stage,
            "persona": persona,
            "agent_run_id": agent_run_id,
            **_redact(data, max_string_length=self.max_content_bytes),
        }
        self._append(self.main_path, event)
        self._append(self.sdk_trace_path, event)
        if stage or persona:
            scope = Path("personas") / persona if persona else Path(stage or "sdk")
            path = self.root / "agents" / scope / "sdk-trace.jsonl"
            path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
            self._append(path, event)
        return event_id

    def _record_usage(self, *, scope: Path, stage: str, persona: str | None,
                      agent_run_id: str, usage: Mapping[str, Any],
                      total_duration_ms: Any, model: Any) -> None:
        aliases = {
            "input": "input_tokens",
            "output": "output_tokens",
            "cache_read": "cache_read_tokens",
            "cache_write": "cache_write_tokens",
            "total": "total_tokens",
            "cost": "cost",
            "turns": "turns",
            "tool_calls": "tool_calls",
        }
        entry = {
            "agent_run_id": agent_run_id,
            "role": stage,
            "persona": persona,
            "model": model,
            **{target: usage.get(source) for source, target in aliases.items()},
            "startup_duration_ms": self._startup_duration_ms.get(agent_run_id),
            "model_duration_ms": usage.get("duration_ms"),
            "total_duration_ms": total_duration_ms,
            "reported": usage.get("reported", True),
        }
        self._usage_entries[agent_run_id] = entry
        usage_path = self.root / "agents" / scope / "usage.json"
        usage_path.write_text(
            json.dumps(entry, ensure_ascii=False, indent=2, default=str) + "\n",
            encoding="utf-8",
        )
        usage_path.chmod(0o600)
        numeric_fields = tuple(aliases.values()) + (
            "startup_duration_ms",
            "model_duration_ms",
            "total_duration_ms",
        )
        totals = {
            field: sum(
                value
                for item in self._usage_entries.values()
                if isinstance((value := item.get(field)), int | float)
            )
            for field in numeric_fields
        }
        aggregate = {
            "run_id": self.run_id,
            "benchmark_id": self.benchmark_id,
            "reported_invocations": sum(
                item.get("reported") is True for item in self._usage_entries.values()
            ),
            "invocation_count": len(self._usage_entries),
            "totals": totals,
            "invocations": list(self._usage_entries.values()),
        }
        aggregate_path = self.root / "usage.json"
        aggregate_path.write_text(
            json.dumps(aggregate, ensure_ascii=False, indent=2, default=str) + "\n",
            encoding="utf-8",
        )
        aggregate_path.chmod(0o600)

    @staticmethod
    def _append(path: Path, value: Mapping[str, Any]) -> None:
        with path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(value, ensure_ascii=False, separators=(",", ":"), default=str) + "\n")

    def timed(self, event_type: str, **kwargs: Any):
        started = time.monotonic()
        self.emit(f"{event_type}.started", **kwargs)

        def finish(**extra: Any) -> None:
            self.emit(f"{event_type}.finished", duration_ms=int((time.monotonic() - started) * 1000), **kwargs, **extra)

        return finish
