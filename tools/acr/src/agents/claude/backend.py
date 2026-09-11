"""Execute ACR invocations with independent Claude Agent SDK clients."""

from __future__ import annotations

import asyncio
import hashlib
import json
import time
import uuid
from collections import deque
from importlib.metadata import version
from pathlib import Path
from typing import TYPE_CHECKING

from ...tools.protocol import ToolBroker
from ..protocol import AgentEvent, AgentRequest, AgentResponse, CancellationToken
from .cli import find_cli, redact
from .protocol import ClaudeAgentError, normalize_usage, validate_result

if TYPE_CHECKING:
    from ...config import RunConfig
    from .session import ClaudeSession


class ClaudeAgentBackend:
    def __init__(self, config: RunConfig, *, repo_root: Path, broker: ToolBroker | None = None):
        self.config = config.validate()
        self.repo_root = repo_root.resolve()
        self.broker = broker
        self._sessions: set[ClaudeSession] = set()
        self._closed = False

    async def run(self, request: AgentRequest, *, event_sink=None, cancellation: CancellationToken | None = None) -> AgentResponse:
        token = cancellation or CancellationToken()
        token.raise_if_cancelled()
        if self._closed:
            raise ClaudeAgentError("Claude backend is closed")
        if request.output_schema is None:
            raise ClaudeAgentError("Claude backend requires a Pydantic output schema")
        try:
            from claude_agent_sdk import ClaudeAgentOptions, ClaudeSDKClient, HookMatcher

            from .session import ClaudeSession
            from .tools import ClaudeTools
        except ImportError as exc:
            raise ClaudeAgentError("Claude Agent SDK is not installed; install tools/acr") from exc
        agent_run_id = str(uuid.uuid4())
        started = time.monotonic()
        usage = {}
        model = None
        session = None
        diagnostics = deque(maxlen=8)

        def emit(kind, *, status="running", **kwargs):
            if event_sink is not None:
                event_sink(AgentEvent(
                    kind, agent_run_id, request.role, request.persona,
                    status=status, **redact(kwargs),
                ))

        def tool_event(event):
            kind = event["event"]
            data = {key: value for key, value in event.items() if key not in {"event", "tool"}}
            status = (
                "error" if kind in {"tool.failed", "tool.denied"}
                else "completed" if kind == "tool.completed"
                else "running"
            )
            emit(kind, tool_name=event["tool"], status=status, data=data)

        try:
            cli_path = find_cli()
            tools = ClaudeTools(request, self.broker, enabled=self.config.tools_enabled, event_hook=tool_event)
            schema = request.output_schema.model_json_schema()
            requested_model = request.model or self.config.model_for(request.role)

            def stderr(line):
                # Never retain an unbounded CLI stderr stream or raw credentials.
                diagnostics.append(redact(line)[:500])

            options = ClaudeAgentOptions(
                tools=[], allowed_tools=tools.allowed_names,
                mcp_servers=tools.servers, strict_mcp_config=True,
                permission_mode="dontAsk", setting_sources=["user", "project", "local"], skills=[],
                plugins=[], agents={},
                system_prompt=request.stable_instructions.text,
                model=None if requested_model == "default" else requested_model,
                effort=self.config.reasoning_effort if request.role == "persona" else self.config.postprocess_reasoning_effort,
                output_format={"type": "json_schema", "schema": schema},
                cwd=self.repo_root, cli_path=cli_path,
                hooks={"PreToolUse": [HookMatcher(hooks=[tools.gate])]},
                max_buffer_size=4 * 1024 * 1024,
                # Safe mode preserves native auth, endpoint and model settings
                # while suppressing filesystem hooks and other customizations.
                # Explicit SDK MCP tools and callbacks remain available.
                extra_args={"safe-mode": None, "no-session-persistence": None, "disable-slash-commands": None},
                stderr=stderr,
            )
            client = ClaudeSDKClient(options=options)
            session = ClaudeSession(client, token)
            self._sessions.add(session)
            emit("started", input_sha256=_sha(request.review_input.content), data={
                "sdk_version": version("claude-agent-sdk"),
                "schema_sha256": _sha(json.dumps(schema, sort_keys=True)),
                "instructions_sha256": _sha(request.stable_instructions.text),
                "cli_path": cli_path,
            })

            async def operation():
                nonlocal usage, model
                await client.query(request.review_input.content)
                result, model = await self._receive(client, tools, emit)
                usage = normalize_usage(result, tool_calls=tools.call_count)
                value = validate_result(result, request.output_schema)
                return value, result

            value, result = await session.run(operation, request.timeout_seconds)
            token.raise_if_cancelled()
            raw = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
            emit("agent.reply", status="completed", output_sha256=_sha(raw), data={
                "output": value,
            })
            emit("completed", status="completed", output_sha256=_sha(raw), usage=usage, data={
                "duration_ms": int((time.monotonic() - started) * 1000),
                "model": model, "session_id": result.session_id,
            })
            return AgentResponse(value=value, agent_run_id=agent_run_id, usage=usage, raw_output=raw)
        except asyncio.CancelledError:
            emit("cancelled", status="cancelled", usage=usage)
            raise
        except Exception as exc:
            detail = redact(str(exc))
            emit("error", status="timeout" if isinstance(exc, TimeoutError) else "error", usage=usage, data={
                "error_type": type(exc).__name__, "error": detail[:1000],
                "diagnostics": list(diagnostics),
                "duration_ms": int((time.monotonic() - started) * 1000),
            })
            raise ClaudeAgentError(f"Claude agent request failed: {detail}") from exc
        finally:
            if session is not None:
                self._sessions.discard(session)

    async def _receive(self, client, tools, emit):
        from claude_agent_sdk import AssistantMessage, ResultMessage, SystemMessage, TextBlock

        initialized = False
        model = None
        async for message in client.receive_response():
            if isinstance(message, SystemMessage) and message.subtype == "init":
                if initialized:
                    raise ClaudeAgentError("Claude returned duplicate init messages")
                tools.validate_init(message.data)
                initialized = True
                model = message.data.get("model")
                emit("agent.reply", data={"cli_version": message.data.get("claude_code_version"), "model": message.data.get("model")})
            elif isinstance(message, AssistantMessage):
                text = "\n".join(block.text for block in message.content if isinstance(block, TextBlock))
                if text:
                    emit("agent.reply", data={"text": text, "model": message.model})
            elif isinstance(message, ResultMessage):
                if not initialized:
                    raise ClaudeAgentError("Claude returned a result before reporting its tools")
                return message, model
        raise ClaudeAgentError("Claude stream ended without a result")

    async def aclose(self) -> None:
        self._closed = True
        sessions = tuple(self._sessions)
        for session in sessions:
            session.stop()
        await asyncio.gather(*(session.wait_closed() for session in sessions))


def _sha(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()
