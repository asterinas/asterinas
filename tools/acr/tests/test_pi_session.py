"""Exercise the real Pi SDK against a local, deterministic model endpoint."""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import tempfile
import threading
import unittest
from dataclasses import replace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from pydantic import BaseModel, ConfigDict

from acr.agents.pi import PiAgentBackend, PiAgentError
from acr.agents.protocol import AgentRequest, ToolSpec
from acr.config import PiConfig, RunConfig
from acr.core.contracts import CommentsEnvelope
from acr.core.invocations import AgentInput, AgentInstructions, AgentInvocation, RunLimits, ToolPolicy
from acr.tools.protocol import ToolBroker, ToolResult


class _Output(BaseModel):
    model_config = ConfigDict(extra="forbid")
    ok: bool


class _SourceProvider:
    def __init__(self):
        self.calls = []

    def specs(self):
        return (ToolSpec(
            "source.read", "Read a source file.",
            {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"], "additionalProperties": False},
            "source",
        ),)

    async def invoke(self, name, arguments):
        self.calls.append((name, arguments))
        return ToolResult(True, "fixture source")


class _Endpoint(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), _Handler)
        self.requests = []
        self.replies = []
        self.entered = threading.Event()
        self.release = threading.Event()


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.server.requests.append(request)
        self.server.entered.set()
        reply = self.server.replies.pop(0) if self.server.replies else {"content": "No structured result."}
        if reply == "wait":
            self.server.release.wait(10)
            reply = {"content": "Stopped."}
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        chunks = [
            {"choices": [{"index": 0, "delta": {"role": "assistant", **reply}, "finish_reason": None}]},
            {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls" if "tool_calls" in reply else "stop"}]},
            {"choices": [], "usage": {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}},
        ]
        try:
            for chunk in chunks:
                payload = {"id": "fixture-response", "object": "chat.completion.chunk", "created": 1, "model": "mock", **chunk}
                self.wfile.write(f"data: {json.dumps(payload)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


def _call(name="structured_output", arguments=None, *, call_id="fixture-call"):
    return {"tool_calls": [{
        "index": 0, "id": call_id, "type": "function",
        "function": {"name": name, "arguments": json.dumps(arguments if arguments is not None else {"ok": True})},
    }]}


@unittest.skipUnless(shutil.which("node") and (shutil.which("pi") or os.environ.get("ACR_PI_SDK_ROOT")), "Pi SDK and Node are optional test dependencies")
class PiSessionTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="acr-pi-session-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.agent_dir = self.root / "agent"
        self.agent_dir.mkdir()
        self.endpoint = _Endpoint()
        thread = threading.Thread(target=self.endpoint.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 5)
        self.addCleanup(self.endpoint.server_close)
        self.addCleanup(self.endpoint.shutdown)
        self.addCleanup(self.endpoint.release.set)
        (self.agent_dir / "models.json").write_text(json.dumps({"providers": {"acr-test": {
            "baseUrl": f"http://127.0.0.1:{self.endpoint.server_port}/v1",
            "api": "openai-completions", "apiKey": "local-test-key",
            "models": [{"id": "mock", "name": "Mock", "reasoning": False, "input": ["text"],
                        "contextWindow": 32768, "maxTokens": 4096,
                        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}}],
        }}}), encoding="utf-8")
        self.config = RunConfig(
            backend="pi-agent", review_model="acr-test/mock", postprocess_model="acr-test/mock",
            pi=PiConfig(agent_dir=self.agent_dir, keep_native_sessions=False, shutdown_grace_seconds=1),
        )
        self.source = _SourceProvider()
        self.backend = PiAgentBackend(self.config, repo_root=self.root, run_root=self.root, broker=ToolBroker(self.source))
        self.addAsyncCleanup(self.backend.aclose)

    def request(self, *, tools=(), timeout=10, output_type=_Output):
        return AgentRequest.from_invocation(AgentInvocation(
            AgentInstructions(("summary",), "ACR_TEST_INSTRUCTIONS", "test-v1"),
            AgentInput.from_text("ACR_TEST_INPUT"), output_type,
            ToolPolicy(allow=tools), RunLimits(timeout_seconds=timeout),
            "summary", None, "acr-test/mock",
        ))

    async def test_direct_session_submits_structured_result_without_subagent_package(self):
        self.endpoint.replies = [_call()]
        # These would contaminate the prompt if ambient resources were loaded.
        (self.root / "AGENTS.md").write_text("PROJECT_SENTINEL", encoding="utf-8")
        (self.agent_dir / "APPEND_SYSTEM.md").write_text("GLOBAL_SENTINEL", encoding="utf-8")
        for directory in (self.root / ".pi" / "extensions", self.agent_dir / "extensions"):
            directory.mkdir(parents=True)
            (directory / "unsafe.ts").write_text('throw new Error("AMBIENT_EXTENSION_LOADED");', encoding="utf-8")
        events = []
        response = await self.backend.run(self.request(), event_sink=events.append)
        self.assertEqual(response.value, {"ok": True})
        reply = next(event for event in events if event.event_type == "agent.reply")
        self.assertEqual(reply.data["output"], {"ok": True})
        self.assertFalse((self.agent_dir / "npm").exists())
        self.assertFalse(list((self.root / "pi-sessions").rglob("*.jsonl")))
        sent = self.endpoint.requests[0]
        self.assertEqual([t["function"]["name"] for t in sent["tools"]], ["structured_output"])
        prompts = json.dumps(sent["messages"])
        self.assertIn("ACR_TEST_INSTRUCTIONS", prompts)
        self.assertNotIn("PROJECT_SENTINEL", prompts)
        self.assertNotIn("GLOBAL_SENTINEL", prompts)
        self.assertEqual(response.usage["input"], 20)
        self.assertEqual(response.usage["output"], 5)
        self.assertEqual(response.usage["turns"], 1)
        self.assertTrue(any(e.event_type == "pi.session.started" for e in events))

    async def test_concurrent_calls_use_independent_sessions(self):
        self.endpoint.replies = [_call(), _call()]
        first, second = await asyncio.gather(
            self.backend.run(self.request()), self.backend.run(self.request()),
        )
        self.assertNotEqual(first.agent_run_id, second.agent_run_id)
        self.assertEqual(first.value, second.value)
        for sent in self.endpoint.requests:
            self.assertEqual([message["role"] for message in sent["messages"]], ["system", "user"])

    async def test_allowlisted_source_tool_still_executes_through_python(self):
        self.endpoint.replies = [_call("source_read", {"path": "fixture.rs"}), _call()]
        response = await self.backend.run(self.request(tools=("source.read",)))
        self.assertEqual(response.value, {"ok": True})
        self.assertEqual(self.source.calls, [("source.read", {"path": "fixture.rs"})])
        self.assertEqual(response.usage["turns"], 2)
        self.assertEqual(response.usage["tool_calls"], 2)
        self.assertIn("fixture source", json.dumps(self.endpoint.requests[1]["messages"]))

    async def test_plain_text_result_is_rejected(self):
        self.endpoint.replies = [{"content": '{"ok": true}'}]
        with self.assertRaisesRegex(PiAgentError, "did not submit a structured result"):
            await self.backend.run(self.request())

    async def test_invalid_structured_arguments_are_not_accepted(self):
        self.endpoint.replies = [_call(arguments={"wrong": True})]
        with self.assertRaisesRegex(PiAgentError, "did not submit a structured result"):
            await self.backend.run(self.request())
        self.assertIn("Validation failed for tool", json.dumps(self.endpoint.requests[1]["messages"]))

    async def test_nested_review_schema_is_validated(self):
        value = {"comments": [{
            "file": "fixture.rs", "line": 1, "persona": "development",
            "grounding": "Fixture defect", "severity": "minor",
            "problem": "A fixture problem.", "fix": "A fixture fix.",
        }]}
        self.endpoint.replies = [_call(arguments=value)]
        response = await self.backend.run(self.request(output_type=CommentsEnvelope))
        self.assertEqual(response.value, value)
        self.assertEqual(CommentsEnvelope.model_validate(response.value).comments[0].line, 1)
        self.assertIn("$defs", self.endpoint.requests[0]["tools"][0]["function"]["parameters"])

    async def test_invalid_nested_review_is_rejected(self):
        self.endpoint.replies = [_call(arguments={"comments": [{"persona": "not-a-persona"}]})]
        with self.assertRaisesRegex(PiAgentError, "did not submit a structured result"):
            await self.backend.run(self.request(output_type=CommentsEnvelope))

    async def test_native_sessions_are_saved_only_when_requested(self):
        self.backend.config = replace(self.config, pi=replace(self.config.pi, keep_native_sessions=True))
        self.endpoint.replies = [_call()]
        await self.backend.run(self.request())
        self.assertTrue(list((self.root / "pi-sessions").rglob("*.jsonl")))

    async def test_result_mixed_with_other_tools_is_rejected(self):
        mixed = _call()
        extra = _call("source_read", {"path": "fixture.rs"})["tool_calls"][0]
        extra.update(index=1, id="second-call")
        mixed["tool_calls"].append(extra)
        self.endpoint.replies = [mixed]
        with self.assertRaisesRegex(PiAgentError, "without other tool calls"):
            await self.backend.run(self.request(tools=("source.read",)))

    async def test_model_failure_is_not_an_empty_result(self):
        # A malformed endpoint response produces a Pi assistant error.
        self.endpoint.replies = [{"tool_calls": [{"index": 0, "type": "function", "function": {"name": "structured_output", "arguments": "not JSON"}}]}]
        with self.assertRaises(PiAgentError):
            await self.backend.run(self.request())

    async def test_long_tool_loop_completes(self):
        self.endpoint.replies = [
            _call("source_read", {"path": "fixture.rs"}, call_id=f"read-{index}")
            for index in range(50)
        ] + [_call()]
        response = await self.backend.run(self.request(tools=("source.read",)))
        self.assertEqual(response.value, {"ok": True})
        self.assertEqual(len(self.source.calls), 50)
        self.assertEqual(response.usage["turns"], 51)

    async def test_timeout_aborts_session(self):
        self.endpoint.replies = ["wait"]
        with self.assertRaisesRegex(PiAgentError, "timeout"):
            await self.backend.run(self.request(timeout=1))

    async def test_cancelling_python_task_stops_bridge(self):
        self.endpoint.replies = ["wait"]
        task = asyncio.create_task(self.backend.run(self.request()))
        entered = await asyncio.to_thread(self.endpoint.entered.wait, 5)
        self.assertTrue(entered)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(self.backend._clients)
