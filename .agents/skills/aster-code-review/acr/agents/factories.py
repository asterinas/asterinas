"""Role-specific request factories.

Factories only construct provider-neutral requests. They do not run agents or
make stage decisions, which keeps the orchestrator's state machine explicit.
"""

from __future__ import annotations

from pathlib import Path

from ..config import RunConfig
from ..core.contracts import CommentsEnvelope, ConsolidationResult, GraderEnvelope, SummaryResult, VerificationEnvelope
from ..core.invocations import AgentInput, AgentInstructions, AgentInvocation, RunLimits, ToolPolicy
from .protocol import AgentRequest


_PROMPT_ROOT = Path(__file__).resolve().parents[1] / "prompts"


def _role_instructions(role: str, version: str) -> AgentInstructions:
    path = _PROMPT_ROOT / f"{role}.md"
    try:
        text = path.read_text(encoding="utf-8").strip()
    except OSError as exc:
        raise RuntimeError(f"cannot load {role} agent prompt: {path}: {exc}") from exc
    if not text:
        raise RuntimeError(f"{role} agent prompt is empty: {path}")
    return AgentInstructions((role,), text, version)


def _request(role: str, instructions: str, input_text: str, output_schema, config: RunConfig, *, persona: str | None = None) -> AgentRequest:
    return AgentRequest(role=role, instructions=instructions, input_text=input_text, output_schema=output_schema, model=config.model, max_turns=config.max_turns, timeout_seconds=config.timeout_seconds, persona=persona)


def persona_request(persona: str, prompt: str, config: RunConfig) -> AgentRequest:
    return _request("persona", f"Review only the {persona} persona scope and follow the pass contract.", prompt, CommentsEnvelope, config, persona=persona)


def verification_request(input_text: str, config: RunConfig) -> AgentRequest:
    return AgentRequest.from_invocation(AgentFactory(config).verification(input_text))


def consolidation_request(input_text: str, config: RunConfig) -> AgentRequest:
    return AgentRequest.from_invocation(AgentFactory(config).consolidation(input_text))


def summary_request(input_text: str, config: RunConfig) -> AgentRequest:
    return AgentRequest.from_invocation(AgentFactory(config).summary(input_text))


def grader_request(input_text: str, config: RunConfig) -> AgentRequest:
    return AgentRequest.from_invocation(AgentFactory(config).grader(input_text))


class AgentFactory:
    """Construct typed invocations without executing an agent."""

    def __init__(self, config: RunConfig):
        self.config = config

    def _policy_for(self, role: str, persona: str | None = None) -> ToolPolicy:
        configured_role = persona if role == "persona" else role
        if configured_role is None:
            raise ValueError("persona tool policy requires a persona")
        return ToolPolicy(allow=self.config.tools.for_agent(configured_role))

    def _policy_for_combined(self, personas: tuple[str, ...]) -> ToolPolicy:
        allowed = {
            tool
            for persona in personas
            for tool in self.config.tools.for_agent(persona)
        }
        return ToolPolicy(allow=tuple(sorted(allowed)))

    def _make(self, role: str, instructions: AgentInstructions, content: str, output_type, *, persona: str | None = None, policy: ToolPolicy | None = None) -> AgentInvocation:
        model = self.config.review_model if role == "grader" and self.config.review_model else self.config.model
        return AgentInvocation(
            instructions=instructions,
            input=AgentInput.from_text(content),
            output_type=output_type,
            tool_policy=policy or self._policy_for(role, persona),
            run_limits=RunLimits(self.config.max_turns, self.config.timeout_seconds, self.config.retries),
            role=role,
            persona=persona,
            model=model,
        )

    def persona(self, instructions: AgentInstructions, review_input: AgentInput, *, persona: str) -> AgentInvocation:
        return AgentInvocation(instructions, review_input, CommentsEnvelope, self._policy_for("persona", persona), RunLimits(self.config.max_turns, self.config.timeout_seconds, self.config.retries), "persona", persona, self.config.model)

    def combined(self, instructions: AgentInstructions, review_input: AgentInput, *, personas: tuple[str, ...]) -> AgentInvocation:
        return AgentInvocation(instructions, review_input, CommentsEnvelope, self._policy_for_combined(personas), RunLimits(self.config.max_turns, self.config.timeout_seconds, self.config.retries), "persona", None, self.config.model)

    def verification(self, content: str) -> AgentInvocation:
        instructions = _role_instructions("verification", "verification-v2")
        return self._make("verification", instructions, content, VerificationEnvelope)

    def consolidation(self, content: str) -> AgentInvocation:
        instructions = _role_instructions("consolidation", "consolidation-v2")
        return self._make("consolidation", instructions, content, ConsolidationResult)

    def summary(self, content: str) -> AgentInvocation:
        instructions = _role_instructions("summary", "summary-v2")
        return self._make("summary", instructions, content, SummaryResult)

    def grader(self, content: str) -> AgentInvocation:
        instructions = _role_instructions("grader", "grader-v2")
        return self._make("grader", instructions, content, GraderEnvelope)
