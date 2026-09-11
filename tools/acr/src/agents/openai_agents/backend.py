"""OpenAI Agents SDK adapter.

SDK-specific integration stays in this package. The rest of ACR consumes the
provider-neutral protocol from :mod:`acr.agents.protocol`.
"""

from __future__ import annotations

import asyncio
import hashlib
import time
import uuid
from typing import Any

from ...config import RunConfig
from ...tools.protocol import ToolBroker
from ..protocol import AgentBackend, AgentEvent, AgentRequest, AgentResponse, CancellationToken
from .protocol import OpenAIAgentError, _dump, _http_error_type, _reasoning_summary, _usage
from .tools import function_tools


class OpenAIAgentsBackend:
    def __init__(self, config: RunConfig, *, broker: ToolBroker | None = None):
        self.config = config.validate()
        self.broker = broker
        if not self.config.provider.api_key:
            raise OpenAIAgentError(
                f"{self.config.provider.api_key_env} is not set; use ACR_BACKEND=fake for model-free runs"
            )

    async def run(self, request: AgentRequest, *, event_sink=None, cancellation: CancellationToken | None = None) -> AgentResponse:
        token = cancellation or CancellationToken()
        token.raise_if_cancelled()
        agent_run_id = str(uuid.uuid4())
        started = time.monotonic()
        self._event(event_sink, AgentEvent("started", agent_run_id, request.role, request.persona, input_sha256=_sha(request.review_input.content), status="running"))
        try:
            run = self._run_sdk(
                request, event_sink=event_sink, agent_run_id=agent_run_id
            )
            if request.timeout_seconds is None:
                result = await run
            else:
                result = await asyncio.wait_for(run, timeout=request.timeout_seconds)
            token.raise_if_cancelled()
            final_output = result.final_output
            raw = final_output if isinstance(final_output, str) else _dump(final_output)
            usage = _usage(result)
            response = AgentResponse(
                value=final_output,
                agent_run_id=agent_run_id,
                usage=usage,
                raw_output=raw,
                reasoning_summary=_reasoning_summary(result),
            )
            self._event(event_sink, AgentEvent("completed", agent_run_id, request.role, request.persona, output_sha256=_sha(raw), usage=usage, status="completed", data={"duration_ms": int((time.monotonic() - started) * 1000), "reasoning": response.reasoning_summary or "unavailable"}))
            return response
        except asyncio.TimeoutError as exc:
            self._event(event_sink, AgentEvent("error", agent_run_id, request.role, request.persona, status="timeout", data={"duration_ms": int((time.monotonic() - started) * 1000)}))
            raise OpenAIAgentError("AGENT_TIMEOUT") from exc
        except asyncio.CancelledError:
            self._event(event_sink, AgentEvent("cancelled", agent_run_id, request.role, request.persona, status="cancelled"))
            raise
        except Exception as exc:
            self._event(event_sink, AgentEvent("error", agent_run_id, request.role, request.persona, status="error", data={"error_type": type(exc).__name__, "error": str(exc)[:500]}))
            raise OpenAIAgentError(f"agent request failed: {exc}") from exc

    async def _run_sdk(self, request: AgentRequest, *, event_sink=None, agent_run_id: str = ""):
        try:
            from agents import Agent, AgentOutputSchema, ModelSettings, Runner, RunConfig as SDKRunConfig, WebSearchTool, custom_span
        except ImportError as exc:  # pragma: no cover - exercised in dependency-free installs
            raise OpenAIAgentError("openai-agents SDK is not installed") from exc
        from .tracing import local_trace_active

        model, resource = self._build_model(request.model or self.config.model_for(request.role))
        reasoning_effort = self._reasoning_effort_for(request.role)
        settings = ModelSettings(
            reasoning={"effort": reasoning_effort},
            include_usage=True,
            retry=self._build_retry_settings(),
        )
        allowed = set(request.invocation.tool_policy.allow) if request.invocation is not None else {spec.name for spec in request.tools}
        tools = self._hosted_tools(request, WebSearchTool=WebSearchTool)
        if self.broker is not None:
            def tool_sink(event: dict[str, Any]) -> None:
                self._event(
                    event_sink,
                    AgentEvent(
                        event.get("event", "tool.event"),
                        agent_run_id,
                        request.role,
                        request.persona,
                        tool_name=event.get("tool"),
                        status=("completed" if event.get("event") == "tool.completed" else "error" if event.get("event") == "tool.failed" else "running"),
                        data={key: value for key, value in event.items() if key not in {"event", "tool"}},
                    ),
                )
            scoped = self.broker.policy(allowed, event_hook=tool_sink)
            tools.extend(function_tools(scoped))
        output_type = request.output_schema
        # OpenAI strict schemas cannot represent arbitrary string-key maps.
        # Consolidation still receives a typed Pydantic result and is parsed
        # fail-closed by the stage; only the provider-side schema strictness is
        # relaxed for this map-shaped contract.
        if request.role == "consolidation" and output_type is not None:
            output_type = AgentOutputSchema(output_type, strict_json_schema=False)
        agent = Agent(
            name=request.role,
            instructions=request.stable_instructions.text,
            model=model,
            model_settings=settings,
            output_type=output_type,
            tools=tools,
        )
        trace_data: dict[str, Any] = {
            "acr_stage": request.role,
            "acr_persona": request.persona,
            "acr_agent_run_id": agent_run_id,
            "acr_model": request.model or self.config.model_for(request.role),
            "acr_input_sha256": _sha(request.review_input.content),
            "acr_instructions_sha256": _sha(request.stable_instructions.text),
            "input": request.review_input.content,
            "instructions": request.stable_instructions.text,
        }
        sdk_run_config = SDKRunConfig(
            tracing_disabled=not (self.config.tracing_enabled and local_trace_active()),
            # EventLogger redacts recognized credentials in these local traces.
            trace_include_sensitive_data=True,
            workflow_name=f"ACR {request.role}",
            trace_metadata={
                "acr_stage": request.role,
                "acr_persona": request.persona or "",
                "acr_agent_run_id": agent_run_id,
            },
        )
        try:
            # The custom span is the stable correlation point for ACR metadata;
            # SDK agent/generation/function spans are nested beneath it.
            with custom_span(
                f"acr.agent.{request.role}",
                data=trace_data,
                disabled=not (self.config.tracing_enabled and local_trace_active()),
            ):
                return await Runner.run(
                    agent,
                    request.review_input.content,
                    # Omitting this argument enables the SDK's default turn cap.
                    max_turns=None,
                    run_config=sdk_run_config,
                )
        finally:
            await resource.close()

    def _hosted_tools(self, request: AgentRequest, *, WebSearchTool=None) -> list[Any]:
        allowed = set(request.invocation.tool_policy.allow) if request.invocation is not None else {spec.name for spec in request.tools}
        if not self.config.tools_enabled or "web.search" not in allowed:
            return []
        if WebSearchTool is None:
            try:
                from agents import WebSearchTool
            except ImportError as exc:  # pragma: no cover
                raise OpenAIAgentError("openai-agents SDK is not installed") from exc
        return [WebSearchTool(search_context_size=self.config.web_search_context_size)]

    def _build_model(self, model_name: str) -> tuple[Any, Any]:
        """Build the TOML-selected SDK model and its closeable resource."""

        adapter = self.config.provider.adapter
        if adapter == "openai":
            from openai import AsyncOpenAI
            from agents.models.openai_chatcompletions import OpenAIChatCompletionsModel
            from agents.models.openai_responses import OpenAIResponsesModel

            client_kwargs: dict[str, Any] = {"api_key": self.config.provider.api_key}
            if self.config.provider.base_url:
                client_kwargs["base_url"] = self.config.provider.base_url
            client = AsyncOpenAI(**client_kwargs)
            if self.config.wire_api == "responses":
                return OpenAIResponsesModel(model_name, openai_client=client), client
            return OpenAIChatCompletionsModel(model_name, openai_client=client), client

        if adapter == "any-llm":
            try:
                from agents.extensions.models.any_llm_model import AnyLLMModel
            except ImportError as exc:
                raise OpenAIAgentError(
                    "the any-llm adapter is not installed; install ACR dependencies "
                    "with openai-agents[any-llm]"
                ) from exc
            from .responses import normalize_responses_input

            class NormalizedAnyLLMModel(AnyLLMModel):
                def _sanitize_any_llm_responses_input(self, list_input: list[Any]) -> list[Any]:
                    cleaned = super()._sanitize_any_llm_responses_input(list_input)
                    return normalize_responses_input(cleaned)

            model = NormalizedAnyLLMModel(
                model=model_name,
                base_url=self.config.provider.base_url,
                api_key=self.config.provider.api_key,
                api=self.config.wire_api,
            )
            return model, model

        # RunConfig.validate() rejects this before execution. Keep the adapter
        # boundary fail-closed if a custom config object bypasses validation.
        raise OpenAIAgentError(f"unsupported provider adapter: {adapter}")

    def _build_retry_settings(self) -> Any:
        from agents import ModelRetrySettings, RetryDecision, retry_policies

        def upstream_http_error(context: Any) -> Any:
            # Some relays report transient upstream failures as bad requests.
            if context.normalized.status_code == 400 and _http_error_type(context.error) == "upstream_error":
                return RetryDecision(retry=True, reason="upstream error reported as HTTP 400")
            return False

        policy = retry_policies.any(
            retry_policies.provider_suggested(),
            retry_policies.network_error(),
            retry_policies.retry_after(),
            retry_policies.http_status((408, 409, 429, 500, 502, 503, 504)),
            upstream_http_error,
        )
        return ModelRetrySettings(
            max_retries=self.config.model_retries,
            backoff={
                "initial_delay": self.config.model_retry_initial_delay,
                "max_delay": self.config.model_retry_max_delay,
                "multiplier": self.config.model_retry_multiplier,
                "jitter": self.config.model_retry_jitter,
            },
            policy=policy,
        )

    def _reasoning_effort_for(self, role: str) -> str:
        if role == "persona":
            return self.config.reasoning_effort
        return self.config.postprocess_reasoning_effort

    @staticmethod
    def _event(sink, event: AgentEvent) -> None:
        if sink is not None:
            sink(event)


def _sha(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()
