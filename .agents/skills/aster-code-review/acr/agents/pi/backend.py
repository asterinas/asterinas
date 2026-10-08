"""Provider-neutral ACR backend implemented with the Pi Agent SDK."""

from __future__ import annotations

import asyncio
import hashlib
import json
import time
import uuid
from typing import Any

from ...config import RunConfig
from ...tools.protocol import ToolBroker
from ..protocol import AgentEvent, AgentRequest, AgentResponse, CancellationToken
from .client import PiBridgeClient
from .protocol import normalize_usage


_DENIED_TOOL_NAMES = {
    "write",
    "edit",
    "bash",
    "powershell",
    "subagent",
    "contact_supervisor",
    "intercom",
}
_READ_ONLY_CAPABILITIES = {"read", "source", "git", "guideline", "rust"}


class PiAgentError(RuntimeError):
    pass


class PiAgentBackend:
    def __init__(
        self,
        config: RunConfig,
        *,
        repo_root,
        run_root,
        broker: ToolBroker | None = None,
    ):
        self.config = config.validate()
        self.repo_root = repo_root.resolve()
        self.run_root = run_root.resolve()
        self.broker = broker
        self._clients: set[PiBridgeClient] = set()

    async def run(
        self,
        request: AgentRequest,
        *,
        event_sink=None,
        cancellation: CancellationToken | None = None,
    ) -> AgentResponse:
        token = cancellation or CancellationToken()
        token.raise_if_cancelled()
        if request.output_schema is None:
            raise PiAgentError("Pi backend requires a Pydantic output schema")
        agent_run_id = str(uuid.uuid4())
        started = time.monotonic()
        schema = request.output_schema.model_json_schema()
        schema_json = json.dumps(
            schema, ensure_ascii=False, sort_keys=True, separators=(",", ":")
        )
        system_prompt = (
            request.stable_instructions.text
            + "\n\n<ACR_PI_OUTPUT_CONTRACT>\n"
            + "Return exactly one JSON value matching the JSON Schema below through "
            + "the structured-output contract. Do not emit Markdown fences or prose "
            + "outside the JSON value. Return the schema-valid empty structure when "
            + "there are no findings.\nJSON_SCHEMA="
            + schema_json
            + "\n</ACR_PI_OUTPUT_CONTRACT>"
        )
        allowed = (
            set(request.invocation.tool_policy.allow)
            if request.invocation is not None
            else {spec.name for spec in request.tools}
        )

        def tool_event(event: dict[str, Any]) -> None:
            self._event(
                event_sink,
                AgentEvent(
                    event.get("event", "tool.event"),
                    agent_run_id,
                    request.role,
                    request.persona,
                    tool_name=event.get("tool"),
                    status=(
                        "completed"
                        if event.get("event") == "tool.completed"
                        else "error"
                        if event.get("event") == "tool.failed"
                        else "running"
                    ),
                    data={
                        key: value
                        for key, value in event.items()
                        if key not in {"event", "tool"}
                    },
                ),
            )

        scoped = self.broker.policy(allowed, event_hook=tool_event) if self.broker else None
        tool_specs = []
        for spec in scoped.specs() if scoped is not None else ():
            sdk_name = spec.name.replace(".", "_")
            if spec.capability not in _READ_ONLY_CAPABILITIES or sdk_name in _DENIED_TOOL_NAMES:
                raise PiAgentError(f"Pi backend rejects non-read-only tool: {spec.name}")
            tool_specs.append(
                {
                    "name": spec.name,
                    "sdk_name": sdk_name,
                    "description": spec.description,
                    "input_schema": spec.input_schema,
                }
            )

        def bridge_event(frame: dict[str, Any]) -> None:
            frame_type = str(frame.get("type", "pi.event"))
            if frame_type in {"result", "error"}:
                return
            data = {key: value for key, value in frame.items() if key != "version"}
            self._event(
                event_sink,
                AgentEvent(
                    f"pi.{frame_type}",
                    agent_run_id,
                    request.role,
                    request.persona,
                    status="running" if frame_type != "diagnostic" else "diagnostic",
                    data=data,
                ),
            )

        self._event(
            event_sink,
            AgentEvent(
                "started",
                agent_run_id,
                request.role,
                request.persona,
                input_sha256=_sha(request.review_input.content),
                status="running",
                data={"schema_sha256": _sha(schema_json), "transport": "pi-sdk"},
            ),
        )
        client = PiBridgeClient(
            self.config.pi,
            broker=scoped,
            session_root=self.run_root / "pi-sessions",
            event_hook=bridge_event,
        )
        self._clients.add(client)
        run_frame = {
            "version": 1,
            "id": agent_run_id,
            "type": "run",
            "request": {
                "role": request.role,
                "persona": request.persona,
                "cwd": str(self.repo_root),
                "model": request.model or self.config.model,
                "thinking": self._thinking_for(request.role),
                "system_prompt": system_prompt,
                "user_prompt": request.review_input.content,
                "output_schema": schema,
                "timeout_ms": (
                    int(request.timeout_seconds * 1000)
                    if request.timeout_seconds is not None
                    else None
                ),
                "tools": tool_specs,
                "tool_timeout_ms": int(
                    (
                        request.invocation.tool_policy.timeout_seconds
                        if request.invocation is not None
                        else 20.0
                    )
                    * 1000
                ),
                "trusted_extensions": list(self.config.pi.trusted_extensions),
                "keep_native_sessions": self.config.pi.keep_native_sessions,
            },
        }
        try:
            execution = client.run(run_frame)
            if request.timeout_seconds is None:
                terminal = await execution
            else:
                terminal = await asyncio.wait_for(
                    execution,
                    timeout=(
                        request.timeout_seconds
                        + self.config.pi.bridge_startup_timeout_seconds
                        + self.config.pi.shutdown_grace_seconds
                    ),
                )
            token.raise_if_cancelled()
            if terminal.get("type") == "error" or terminal.get("status") != "completed":
                raise PiAgentError(
                    str(terminal.get("error") or f"Pi child status: {terminal.get('status')}")
                )
            value = terminal.get("value")
            raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
            usage = normalize_usage(terminal.get("usage"))
            self._event(
                event_sink,
                AgentEvent(
                    "agent.reply",
                    agent_run_id,
                    request.role,
                    request.persona,
                    output_sha256=_sha(raw),
                    status="completed",
                    data={
                        "output": value
                        if self.config.trace_include_sensitive_data
                        else f"[omitted sha256={_sha(raw)} length={len(raw)}]"
                    },
                ),
            )
            self._event(
                event_sink,
                AgentEvent(
                    "completed",
                    agent_run_id,
                    request.role,
                    request.persona,
                    output_sha256=_sha(raw),
                    usage=usage,
                    status="completed",
                    data={
                        "duration_ms": int((time.monotonic() - started) * 1000),
                        "model": terminal.get("model"),
                        "launch_contract_digest": terminal.get(
                            "launch_contract_digest"
                        ),
                    },
                ),
            )
            return AgentResponse(
                value=value,
                agent_run_id=agent_run_id,
                usage=usage,
                raw_output=raw,
            )
        except asyncio.CancelledError:
            self._event(
                event_sink,
                AgentEvent(
                    "cancelled",
                    agent_run_id,
                    request.role,
                    request.persona,
                    status="cancelled",
                ),
            )
            raise
        except Exception as exc:
            self._event(
                event_sink,
                AgentEvent(
                    "error",
                    agent_run_id,
                    request.role,
                    request.persona,
                    status="error",
                    data={"error_type": type(exc).__name__, "error": str(exc)[:1000]},
                ),
            )
            if isinstance(exc, PiAgentError):
                raise
            raise PiAgentError(f"Pi agent request failed: {exc}") from exc
        finally:
            self._clients.discard(client)

    async def aclose(self) -> None:
        await asyncio.gather(*(client.close() for client in tuple(self._clients)))
        self._clients.clear()

    def _thinking_for(self, role: str) -> str:
        return (
            self.config.reasoning_effort
            if role == "persona"
            else self.config.postprocess_reasoning_effort
        )

    @staticmethod
    def _event(sink, event: AgentEvent) -> None:
        if sink is not None:
            sink(event)


def _sha(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()
