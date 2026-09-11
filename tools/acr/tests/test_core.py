from __future__ import annotations

import asyncio
import inspect
import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

from acr.agents.factories import AgentFactory
from acr.agents.openai_agents import OpenAIAgentsBackend, _http_error_type
from acr.agents.openai_agents.protocol import OpenAIAgentError
from acr.agents.fake import FakeBackend
from acr.agents.output_parser import OutputParseError, parse_comments
from acr.agents.protocol import AgentRequest
from acr.config import ConfigError, ProviderConfig, RunConfig, ToolConfig, load_config
from acr.core.activation import DeterministicActivationPolicy
from acr.core.contracts import CommentsEnvelope, ReviewComment
from acr.core.events import EventLogger
from acr.core.invocations import AgentInput, AgentInstructions
from acr.core.scheduler import PassScheduler
from acr.core.state import RunContext
from acr.core.assemble import AssemblyError, ReviewDocument, assemble_fragments, render_markdown
from acr.core.consolidate import consolidate_document
from acr.core.prompts import InstructionCompiler
from acr.core.summary import summarize_document
from acr.core.verify import comment_id, verify_document


def comment(persona="development", line=4, problem="bad", fix="fix"):
    return {
        "file": "kernel/src/foo.rs",
        "line": line,
        "persona": persona,
        "grounding": "Off by one",
        "severity": "major",
        "problem": problem,
        "fix": fix,
    }


class CoreTests(unittest.TestCase):
    def test_explicit_config_is_the_only_toml_loaded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            selected = root / "selected.toml"
            selected.write_text(
                '[agent.persona_review]\nmodel = "selected-review"\n'
                '[agent.postprocess]\nmodel = "selected-postprocess"\n',
                encoding="utf-8",
            )
            (root / "acr.openai.toml").write_text(
                '[agent.persona_review]\nmodel = "unselected-review"\n',
                encoding="utf-8",
            )
            with patch.dict(os.environ, {}, clear=True):
                config = load_config(selected)
            self.assertEqual(config.postprocess_model, "selected-postprocess")
            self.assertEqual(config.review_model, "selected-review")
            self.assertEqual(config.reasoning_effort, RunConfig().reasoning_effort)
            self.assertEqual(config.tools, ToolConfig.builtin(web_search=True))

    def test_explicit_config_must_exist(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing.toml"
            with self.assertRaisesRegex(ConfigError, "config file does not exist"):
                load_config(missing)

    def test_only_the_profile_selects_the_backend(self):
        with patch.dict(os.environ, {}, clear=True):
            default = load_config()
            fake = load_config(Path(__file__).resolve().parent / "acr.fake.toml")
        self.assertEqual(default.backend, "openai-agents")
        self.assertEqual((fake.backend, fake.review_model), ("fake", RunConfig().review_model))
        with patch.dict(os.environ, {"ACR_BACKEND": "pi-agent"}, clear=True):
            with self.assertRaisesRegex(ConfigError, "ACR_BACKEND is no longer supported; use --config"):
                load_config()

    def test_settings_follow_cli_then_environment_then_profile(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.toml"
            path.write_text(
                '[agent]\nbackend = "fake"\nper_persona_context = "yes"\n'
                '[agent.persona_review]\nmodel = "profile-model"\neffort = "low"\n',
                encoding="utf-8",
            )
            env = {"ACR_PER_PERSONA_CONTEXT": "auto", "ACR_PERSONA_REVIEW_MODEL": "env-model"}
            with patch.dict(os.environ, env, clear=True):
                config = load_config(path, overrides={"agent.per_persona_context": "no"})
            self.assertEqual(config.per_persona_context, "no")
            self.assertEqual(config.review_model, "env-model")
            self.assertEqual(config.reasoning_effort, "low")
            self.assertEqual((config.max_concurrency, config.retries, config.model_retries), (5, 2, 6))
            self.assertIsNone(config.timeout_seconds)

    def test_retired_settings_have_migration_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "legacy.toml"
            for text, message in (
                ('[agent]\nreview_model = "x"\n', "agent.persona_review.model"),
                ('[tools]\nsecurity = []\n', "built in"),
                ('[benchmark]\nremote = "https://example.com/x"\n', "upstream"),
                ('[provider]\nadapter = "openai"\n', r"\[openai\]"),
                ('[agent]\nreview_modle = "x"\n', "unknown setting agent.review_modle"),
                ('[agent]\nretries = 1\n', "at most 3 times"),
                ('[agent]\ntimeout_seconds = 60\n', "no time limit"),
                ('[agent]\nmax_concurrency = 6\n', "at most 5 agents"),
                ('[openai]\nweb_search = true\n', "built in"),
                ('[openai]\nmodel_retries = 6\n', "built in"),
            ):
                with self.subTest(text=text):
                    path.write_text(text, encoding="utf-8")
                    with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(ConfigError, message):
                        load_config(path)
            for name, message in (("ACR_REASONING_EFFORT", "ACR_PERSONA_REVIEW_EFFORT"), ("ACR_RETRIES", "built in")):
                with self.subTest(env=name), patch.dict(os.environ, {name: "1"}, clear=True):
                    with self.assertRaisesRegex(ConfigError, message):
                        load_config(overrides={"agent.backend": "fake"})

    def test_config_rejects_unknown_wire_api(self):
        with self.assertRaises(ConfigError):
            RunConfig(wire_api="messages").validate()

    def test_credentials_come_only_from_fixed_environment_variables(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.toml"
            path.write_text('[openai]\nadapter = "any-llm"\n', encoding="utf-8")
            with patch.dict(os.environ, {"OPENAI_API_KEY": "legacy", "OPENAI_BASE_URL": "https://legacy.example/v1"}, clear=True):
                config = load_config(path)
                self.assertIsNone(config.provider.api_key)
                self.assertIsNone(config.provider.base_url)
                with self.assertRaisesRegex(OpenAIAgentError, r"ACR_API_KEY is not set \(OPENAI_API_KEY is no longer read\)"):
                    OpenAIAgentsBackend(config)
                with patch.dict(os.environ, {"ACR_API_KEY": "test-only", "ACR_BASE_URL": " https://relay.example/v1 "}):
                    self.assertEqual(config.provider.api_key, "test-only")
                    self.assertEqual(config.provider.base_url, "https://relay.example/v1")
            path.write_text('[openai]\napi_key_env = "MY_KEY"\n', encoding="utf-8")
            with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(ConfigError, "always read from ACR_API_KEY"):
                load_config(path)

    def test_config_accepts_supported_provider_adapters(self):
        RunConfig(provider=ProviderConfig(adapter="openai")).validate()
        RunConfig(
            wire_api="chat_completions",
            provider=ProviderConfig(adapter="any-llm"),
        ).validate()
        with self.assertRaises(ConfigError):
            RunConfig(provider=ProviderConfig(adapter="custom")).validate()

    def test_config_rejects_unknown_postprocess_reasoning_effort(self):
        with self.assertRaises(ConfigError):
            RunConfig(postprocess_reasoning_effort="minimal").validate()

    def test_agent_timeout_is_optional_but_positive_when_set(self):
        self.assertIsNone(RunConfig().validate().timeout_seconds)
        RunConfig(timeout_seconds=600).validate()
        with self.assertRaises(ConfigError):
            RunConfig(timeout_seconds=0).validate()

    def test_web_search_requires_responses_and_valid_context_size(self):
        RunConfig(wire_api="chat_completions").validate()
        RunConfig(
            wire_api="chat_completions",
            tools_enabled=False,
            tools=ToolConfig(verification=("web.search",)),
        ).validate()
        with self.assertRaises(ConfigError):
            RunConfig(
                wire_api="chat_completions",
                tools=ToolConfig(verification=("web.search",)),
            ).validate()
        with self.assertRaises(ConfigError):
            RunConfig(web_search_context_size="huge").validate()
        with self.assertRaises(ConfigError):
            RunConfig(
                tools=ToolConfig(development=("source.read", "source.read"))
            ).validate()

    def test_web_search_follows_backend_and_wire_api(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.toml"
            for text, expected in (
                ('[openai]\nwire_api = "responses"\n', True),
                ('[openai]\nwire_api = "chat_completions"\n', False),
                ('[agent]\nbackend = "pi-agent"\n[openai]\nwire_api = "responses"\n', False),
            ):
                with self.subTest(text=text):
                    path.write_text(text, encoding="utf-8")
                    with patch.dict(os.environ, {}, clear=True):
                        self.assertEqual(load_config(path).tools.uses("web.search"), expected)

    def test_web_search_is_dropped_for_models_the_provider_rejects(self):
        from acr.core.orchestrator import drop_unsupported_web_search

        class ProbeBackend:
            def __init__(self, refused):
                self.refused = refused
                self.probes = []

            async def web_search_refusal(self, model, effort):
                self.probes.append((model, effort))
                return "HTTP 400: unsupported tool" if model in self.refused else None

        config = RunConfig(
            review_model="review", postprocess_model="post",
            tools=ToolConfig.builtin(web_search=True),
        )
        with tempfile.TemporaryDirectory() as directory:
            logger = EventLogger(Path(directory), "run")
            for refused, expected in (
                (set(), ("maintainability", "development", "verification")),
                ({"review"}, ("verification",)),
                ({"review", "post"}, ()),
            ):
                with self.subTest(refused=refused):
                    backend = ProbeBackend(refused)
                    result = asyncio.run(drop_unsupported_web_search(backend, config, logger))
                    self.assertEqual(result.tools.roles_using("web.search"), expected)
                    # Maintainability and Development share one persona model.
                    self.assertEqual(backend.probes, [("review", "high"), ("post", "low")])
                    self.assertIn("source.read", result.tools.development)
            unprobed = asyncio.run(drop_unsupported_web_search(FakeBackend(), config, logger))
            self.assertIs(unprobed, config)

    def test_model_retry_config_validation(self):
        RunConfig(model_retries=0).validate()
        with self.assertRaises(ConfigError):
            RunConfig(model_retries=-1).validate()
        with self.assertRaises(ConfigError):
            RunConfig(model_retry_initial_delay=2.0, model_retry_max_delay=1.0).validate()

    def test_http_error_type_uses_structured_body(self):
        error = RuntimeError("request failed")
        error.body = {"error": {"type": "upstream_error"}}
        self.assertEqual(_http_error_type(error), "upstream_error")
        error.body = {"error": {"message": "upstream_error only in text"}}
        self.assertIsNone(_http_error_type(error))

    def test_model_retry_policy_recognizes_transient_upstream_errors(self):
        from agents import ModelRetryNormalizedError, RetryPolicyContext

        with patch.dict(os.environ, {"ACR_TEST_API_KEY": "test-only"}):
            backend = OpenAIAgentsBackend(RunConfig(
                provider=ProviderConfig(api_key_env="ACR_TEST_API_KEY"),
            ))
        policy = backend._build_retry_settings().policy

        async def decision_for(status, error_type):
            error = RuntimeError("request failed")
            error.body = {"error": {"type": error_type}}
            context = RetryPolicyContext(
                error=error,
                attempt=1,
                max_retries=3,
                stream=False,
                normalized=ModelRetryNormalizedError(status_code=status),
            )
            value = policy(context)
            return await value if inspect.isawaitable(value) else value

        for status, error_type, retry in (
            (400, "upstream_error", True),
            (400, "invalid_request_error", False),
            (400, "authentication_error", False),
            (401, "upstream_error", False),
            (429, "rate_limit_error", True),
            (502, "upstream_error", True),
        ):
            with self.subTest(status=status, error_type=error_type):
                self.assertEqual(asyncio.run(decision_for(status, error_type)).retry, retry)

    def test_sdk_model_adapter_selection(self):
        with patch.dict(os.environ, {
            "ACR_TEST_API_KEY": "test-only", "ACR_TEST_BASE_URL": "https://relay.example/v1",
        }):
            any_llm_config = RunConfig(
                wire_api="chat_completions",
                provider=ProviderConfig(
                    adapter="any-llm",
                    api_key_env="ACR_TEST_API_KEY",
                    base_url_env="ACR_TEST_BASE_URL",
                ),
            )
            any_llm_backend = OpenAIAgentsBackend(any_llm_config)
            self.assertEqual(any_llm_backend._reasoning_effort_for("persona"), "high")
            self.assertEqual(any_llm_backend._reasoning_effort_for("verification"), "low")
            any_llm_model, any_llm_resource = any_llm_backend._build_model("openai/test-model")
            self.assertEqual(type(any_llm_model).__name__, "NormalizedAnyLLMModel")
            self.assertEqual(any_llm_model.api, "chat_completions")
            self.assertEqual(any_llm_model.base_url, "https://relay.example/v1")
            retry = any_llm_backend._build_retry_settings()
            self.assertEqual(retry.max_retries, 6)
            asyncio.run(any_llm_resource.close())

            openai_config = RunConfig(
                provider=ProviderConfig(
                    adapter="openai",
                    api_key_env="ACR_TEST_API_KEY",
                    base_url_env="ACR_TEST_BASE_URL",
                ),
            )
            openai_backend = OpenAIAgentsBackend(openai_config)
            openai_model, openai_resource = openai_backend._build_model("test-model")
            self.assertEqual(type(openai_model).__name__, "OpenAIResponsesModel")
            self.assertEqual(str(openai_resource.base_url), "https://relay.example/v1/")
            asyncio.run(openai_resource.close())

    def test_openai_agent_can_exceed_the_sdk_default_turn_limit(self):
        from agents import function_tool
        from agents.items import ModelResponse
        from agents.models.interface import Model
        from agents.run import DEFAULT_MAX_TURNS
        from agents.usage import Usage
        from openai.types.responses import ResponseFunctionToolCall, ResponseOutputMessage, ResponseOutputText

        class ReviewModel(Model):
            calls = 0

            async def get_response(self, *_args, **_kwargs):
                self.calls += 1
                if self.calls <= DEFAULT_MAX_TURNS:
                    output = ResponseFunctionToolCall(
                        type="function_call", name="continue_review", arguments="{}",
                        call_id=f"review-{self.calls}",
                    )
                else:
                    output = ResponseOutputMessage(
                        id="review-result", type="message", role="assistant", status="completed",
                        content=[ResponseOutputText(type="output_text", text='{"comments": []}', annotations=[])],
                    )
                return ModelResponse(output=[output], usage=Usage(requests=1), response_id=None)

            def stream_response(self, *_args, **_kwargs):
                raise AssertionError("streaming is not used in this test")

        @function_tool
        def continue_review() -> str:
            return "Continue investigating."

        with patch.dict(os.environ, {"ACR_TEST_API_KEY": "test-only"}):
            backend = OpenAIAgentsBackend(RunConfig(
                provider=ProviderConfig(api_key_env="ACR_TEST_API_KEY"),
                tracing_enabled=False,
            ))
        model = ReviewModel()
        with (
            patch.object(backend, "_build_model", return_value=(model, model)),
            patch.object(backend, "_hosted_tools", return_value=[continue_review]),
        ):
            response = asyncio.run(backend.run(AgentRequest("persona", "Review the code.", "code", CommentsEnvelope)))
        self.assertEqual(response.value.model_dump(), {"comments": []})
        self.assertEqual(model.calls, DEFAULT_MAX_TURNS + 1)

    def test_web_search_probe_reports_provider_refusal(self):
        from agents import WebSearchTool
        from agents.items import ModelResponse
        from agents.models.interface import Model
        from agents.usage import Usage
        from openai.types.responses import ResponseOutputMessage, ResponseOutputText

        class ProbeModel(Model):
            def __init__(self, error=None):
                self.error = error
                self.tools = None

            async def get_response(self, *_args, model_settings, tools, **_kwargs):
                self.tools = tools
                self.effort = model_settings.reasoning.effort
                if self.error is not None:
                    raise self.error
                output = ResponseOutputMessage(
                    id="probe", type="message", role="assistant", status="completed",
                    content=[ResponseOutputText(type="output_text", text="OK", annotations=[])],
                )
                return ModelResponse(output=[output], usage=Usage(requests=1), response_id=None)

            def stream_response(self, *_args, **_kwargs):
                raise AssertionError("streaming is not used in this test")

        with patch.dict(os.environ, {"ACR_TEST_API_KEY": "test-only"}):
            backend = OpenAIAgentsBackend(RunConfig(
                provider=ProviderConfig(api_key_env="ACR_TEST_API_KEY"),
                tracing_enabled=False,
            ))
        for error, expected in ((None, None), (ValueError("web_search is not supported"), "ValueError: web_search is not supported")):
            with self.subTest(error=error):
                model = ProbeModel(error)
                with patch.object(backend, "_build_model", return_value=(model, model)):
                    refusal = asyncio.run(backend.web_search_refusal("probe-model", "medium"))
                self.assertEqual(refusal, expected)
                self.assertEqual([type(tool) for tool in model.tools], [WebSearchTool])
                self.assertEqual(model.effort, "medium")

    def test_postprocess_prompts_and_tool_boundaries(self):
        factory = AgentFactory(RunConfig(backend="fake", tools=ToolConfig.builtin(web_search=True)))
        verification = factory.verification("{}")
        self.assertIn("confident refutation", verification.instructions.text)
        self.assertIn("web.search", verification.tool_policy.allow)
        self.assertEqual(verification.instructions.contract_version, "verification-v2")

        consolidation = factory.consolidation("{}")
        self.assertIn("Every symptom must remain", consolidation.instructions.text)
        self.assertEqual(consolidation.tool_policy.allow, ())

        summary = factory.summary("{}")
        self.assertIn("severity order", summary.instructions.text)
        self.assertEqual(summary.tool_policy.allow, ())

        grader = factory.grader("{}")
        self.assertIn("MATCH IF", grader.instructions.text)
        self.assertEqual(grader.tool_policy.allow, ())

    def test_web_search_is_exposed_only_to_configured_roles(self):
        class StubWebSearchTool:
            def __init__(self, **kwargs):
                self.kwargs = kwargs

        configured_tools = ToolConfig.builtin(web_search=True)
        with patch.dict(os.environ, {"ACR_TEST_API_KEY": "test-only"}):
            backend = OpenAIAgentsBackend(RunConfig(
                provider=ProviderConfig(api_key_env="ACR_TEST_API_KEY"),
                web_search_context_size="high",
                tools=configured_tools,
            ))
        factory = AgentFactory(backend.config)
        verification_tools = backend._hosted_tools(
            AgentRequest.from_invocation(factory.verification("{}")),
            WebSearchTool=StubWebSearchTool,
        )
        self.assertEqual(len(verification_tools), 1)
        self.assertEqual(verification_tools[0].kwargs, {"search_context_size": "high"})

        def persona_request(persona):
            instructions = AgentInstructions((persona,), f"{persona} instructions", "test")
            invocation = factory.persona(
                instructions,
                AgentInput.from_text("review input"),
                persona=persona,
            )
            return AgentRequest.from_invocation(invocation)

        for persona in ("development", "maintainability"):
            self.assertEqual(
                len(backend._hosted_tools(
                    persona_request(persona),
                    WebSearchTool=StubWebSearchTool,
                )),
                1,
            )
        for persona in ("security", "hardware", "documentation"):
            request = persona_request(persona)
            self.assertEqual(
                backend._hosted_tools(request, WebSearchTool=StubWebSearchTool),
                [],
            )
            self.assertFalse(
                {"web.search", "linux.search", "linux.read", "man.search", "man.read"}
                & set(request.invocation.tool_policy.allow)
            )

        combined = factory.combined(
            AgentInstructions(
                ("development", "security"),
                "combined instructions",
                "test",
            ),
            AgentInput.from_text("review input"),
            personas=("development", "security"),
        )
        self.assertEqual(
            len(backend._hosted_tools(
                AgentRequest.from_invocation(combined),
                WebSearchTool=StubWebSearchTool,
            )),
            1,
        )
        grader_tools = backend._hosted_tools(
            AgentRequest.from_invocation(factory.grader("{}")),
            WebSearchTool=StubWebSearchTool,
        )
        self.assertEqual(grader_tools, [])

    def test_factories_use_tool_allowlists_from_config(self):
        factory = AgentFactory(RunConfig(backend="fake", tools=ToolConfig(
            development=("source.read", "web.search"),
            security=("git.show",),
            verification=("source.list",),
        )))
        instructions = AgentInstructions(("development",), "instructions", "test")
        review_input = AgentInput.from_text("review input")
        development = factory.persona(
            instructions,
            review_input,
            persona="development",
        )
        self.assertEqual(development.tool_policy.allow, ("source.read", "web.search"))
        self.assertEqual(factory.verification("{}").tool_policy.allow, ("source.list",))

        combined = factory.combined(
            AgentInstructions(
                ("development", "security"),
                "combined instructions",
                "test",
            ),
            review_input,
            personas=("development", "security"),
        )
        self.assertEqual(
            combined.tool_policy.allow,
            ("git.show", "source.read", "web.search"),
        )

    def test_activation_matches_path_rules(self):
        policy = DeterministicActivationPolicy()
        self.assertEqual(policy.for_paths(["kernel/src/foo.rs"]), ("maintainability", "development", "security"))
        self.assertIn("hardware", policy.for_paths(["ostd/src/arch/x86/trap.S"]))
        self.assertIn("documentation", policy.for_paths(["book/src/guide.md"]))

    def test_contract_rejects_bad_fragment(self):
        self.assertEqual(parse_comments({"comments": []}), [])
        with self.assertRaises(OutputParseError):
            parse_comments([comment() | {"severity": "unknown"}])

    def test_sdk_pass_contract_delegates_structure_to_output_type(self):
        from agents import AgentOutputSchema

        contract = InstructionCompiler().contract
        self.assertIn("guideline.show", contract)
        self.assertNotIn("python3 ", contract)
        self.assertNotIn("```json", contract)
        self.assertNotIn("JSON array", contract)

        output_schema = AgentOutputSchema(CommentsEnvelope)
        self.assertTrue(output_schema.is_strict_json_schema())
        schema = output_schema.json_schema()
        self.assertEqual(schema["required"], ["comments"])
        comment_schema = schema["$defs"]["ReviewComment"]
        self.assertFalse(comment_schema["additionalProperties"])
        self.assertEqual(
            set(comment_schema["required"]),
            {"file", "line", "persona", "grounding", "severity", "problem", "fix", "diff"},
        )

    def test_development_instructions_require_diff_local_risk_sweep(self):
        source_root = Path(__file__).resolve().parents[1] / "src"
        with patch(
            "acr.core.prompts._guideline_catalog",
            return_value="GUIDELINE_CATALOG persona=development digest=test-digest",
        ):
            instructions = InstructionCompiler(source_root=source_root).compile_persona(
                "development", guideline_root=source_root
            )
        self.assertIn("mandatory diff-local risk sweep", instructions.text)
        self.assertIn("Resource lifetime", instructions.text)
        self.assertIn("Removal identity and cardinality", instructions.text)
        self.assertIn("Wait-path cleanup", instructions.text)
        self.assertIn("revisit each matching high-risk statement", instructions.text)

    def test_assembly_is_fail_closed_and_dedups_within_persona(self):
        fragments = {"development": [comment(), comment()], "security": []}
        document = assemble_fragments({"mode": "files", "files": "kernel/src/foo.rs"}, fragments, ("development", "security"))
        self.assertEqual(len(document.comments), 1)
        with self.assertRaises(AssemblyError):
            assemble_fragments({}, {"development": "not-an-array"}, ("development",))
        self.assertIn("<!-- SUMMARY -->", render_markdown(document))

    def test_scheduler_fan_out_and_combined(self):
        async def run():
            backend = FakeBackend()
            prompts = {"development": "===== REVIEW INPUT =====\ninput", "security": "===== REVIEW INPUT =====\ninput"}
            fan = PassScheduler(backend, RunConfig(backend="fake", per_persona_context="yes"))
            result = await fan.run(prompts, ("development", "security"))
            self.assertEqual(result, {"development": [], "security": []})
            self.assertEqual(len(backend.requests), 2)
            backend.requests.clear()
            combined = PassScheduler(backend, RunConfig(backend="fake", per_persona_context="no"))
            await combined.run(prompts, ("development", "security"))
            self.assertEqual(len(backend.requests), 1)
        asyncio.run(run())

    def test_verification_uncertain_marks_and_refuted_retracts(self):
        first = ReviewComment.model_validate(comment(line=4))
        second = ReviewComment.model_validate(comment(line=8, problem="other"))
        document = ReviewDocument({}, [first, second])
        responses = {
            "verification": lambda request: {"items": [
                {"comment_id": comment_id(first, 0), "verdict": "uncertain", "premise": "p", "evidence": [], "reason": "could not verify"},
                {"comment_id": comment_id(second, 1), "verdict": "refuted", "premise": "p", "evidence": [], "reason": "premise is false"},
            ]}
        }
        async def run():
            result = await verify_document(document, FakeBackend(responses), RunConfig(backend="fake"))
            self.assertEqual(len(result.comments), 1)
            self.assertTrue(result.comments[0].problem.startswith("(unverified) "))
            self.assertEqual(len(result.retracted), 1)
        asyncio.run(run())

    def test_postprocess_retries_execution_and_validation_failures(self):
        async def run():
            config = RunConfig(backend="fake", retries=1)
            finding = ReviewComment.model_validate(comment())
            document = ReviewDocument({}, [finding])
            finding_id = comment_id(finding, 0)

            verification_attempts = 0

            def verification_response(_request):
                nonlocal verification_attempts
                verification_attempts += 1
                if verification_attempts == 1:
                    return {"items": []}
                return {"items": [{
                    "comment_id": finding_id,
                    "verdict": "confirmed",
                    "premise": "p",
                    "evidence": [],
                    "reason": "confirmed",
                }]}

            verification_backend = FakeBackend({"verification": verification_response})
            await verify_document(document, verification_backend, config)
            self.assertEqual(len(verification_backend.requests), 2)

            consolidation_attempts = 0

            def consolidation_response(_request):
                nonlocal consolidation_attempts
                consolidation_attempts += 1
                if consolidation_attempts == 1:
                    return {"shared_fixes": {}, "comment_updates": {"unknown": "fix"}}
                return {"shared_fixes": {}, "comment_updates": {}}

            consolidation_backend = FakeBackend({"consolidation": consolidation_response})
            await consolidate_document(document, consolidation_backend, config)
            self.assertEqual(len(consolidation_backend.requests), 2)

            summary_attempts = 0

            def summary_response(_request):
                nonlocal summary_attempts
                summary_attempts += 1
                if summary_attempts == 1:
                    raise RuntimeError("agent failed")
                return {"summary": "Summary after retry."}

            summary_backend = FakeBackend({"summary": summary_response})
            result = await summarize_document(document, summary_backend, config)
            self.assertEqual(result.summary, "Summary after retry.")
            self.assertEqual(len(summary_backend.requests), 2)

        asyncio.run(run())

    def test_event_logger_redacts_secrets(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            logger = EventLogger(root, "run")
            logger.emit(
                "test",
                api_key="do-not-write",
                nested={"authorization": "also-secret"},
                usage={"input_tokens": 123, "output_tokens": 45, "total_tokens": 168},
            )
            text = (root / "main.jsonl").read_text()
            self.assertNotIn("do-not-write", text)
            self.assertNotIn("also-secret", text)
            self.assertIn("[REDACTED]", text)
            self.assertIn('"input_tokens":123', text)
            self.assertIn('"output_tokens":45', text)

    def test_event_logger_keeps_tool_events_in_jsonl_without_console_spam(self):
        with tempfile.TemporaryDirectory() as temp:
            logger = EventLogger(Path(temp), "run")
            console = io.StringIO()
            with redirect_stderr(console):
                logger.agent(
                    "tool.started",
                    stage="persona",
                    persona="development",
                    agent_run_id="agent-1",
                    tool_name="source.read",
                )
                logger.agent(
                    "started",
                    stage="persona",
                    persona="development",
                    agent_run_id="agent-1",
                )
            self.assertNotIn("source.read", console.getvalue())
            self.assertEqual(
                console.getvalue().strip(),
                "[acr] persona started persona=development",
            )
            self.assertIn('"event_type":"tool.started"', (Path(temp) / "main.jsonl").read_text())


if __name__ == "__main__":
    unittest.main()
