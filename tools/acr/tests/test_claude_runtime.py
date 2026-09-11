"""Exercise the real SDK/CLI against a local fake API, without account usage."""

from __future__ import annotations

import asyncio
import importlib.util
import json
import os
import shlex
import shutil
import subprocess
import tempfile
import threading
import unittest
from dataclasses import replace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

from acr.agents.backend_factory import create_backend
from acr.agents.claude.protocol import ClaudeAgentError
from acr.agents.claude.tools import ClaudeTools
from acr.agents.factories import AgentFactory
from acr.agents.protocol import AgentRequest, CancellationToken
from acr.config import RunConfig, ToolConfig, load_config
from acr.core.contracts import CommentsEnvelope
from acr.core.invocations import AgentInput, AgentInstructions
from acr.core.orchestrator import run_review


SDK = importlib.util.find_spec("claude_agent_sdk")
CLI = shutil.which("claude")
if CLI is None and SDK is not None:
    bundled = Path(SDK.origin).parent / "_bundled" / "claude"
    CLI = str(bundled) if bundled.is_file() else None
HAS_CLI = SDK is not None and CLI is not None


class LocalAPI:
    def __init__(self):
        self.requests = []
        self.headers = []
        self.started = threading.Event()
        self.release = threading.Event()
        self.hang = False
        api = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["content-length"])))
                api.requests.append(body)
                api.headers.append(dict(self.headers))
                api.started.set()
                if api.hang:
                    api.release.wait(15)
                try:
                    self.reply(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def reply(self, body):
                names = [tool["name"] for tool in body.get("tools", [])]
                has_read = any(
                    block.get("type") == "tool_use" and block.get("name") == "mcp__acr__source_read"
                    for message in body.get("messages", [])
                    for block in message.get("content", []) if isinstance(block, dict)
                )
                if "mcp__acr__source_read" in names and not has_read:
                    name = "mcp__acr__source_read"
                    value = {"ranges": [{"path": "hello.txt", "start": 1, "end": 1}]}
                else:
                    name = "StructuredOutput"
                    tool = next(tool for tool in body["tools"] if tool["name"] == name)
                    properties = tool["input_schema"]["properties"]
                    if "summary" in properties:
                        value = {"summary": "Local simulated review."}
                    elif "shared_fixes" in properties:
                        value = {"shared_fixes": {"root": "Fix the root cause."}, "comment_updates": {}}
                    else:
                        value = {key: [] for key in properties}
                message = {
                    "id": "msg_local", "type": "message", "role": "assistant",
                    "model": body["model"], "content": [], "stop_reason": None,
                    "stop_sequence": None, "usage": {"input_tokens": 2, "output_tokens": 1},
                }
                block = {"type": "tool_use", "id": f"call_{len(api.requests)}", "name": name, "input": {}}
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.end_headers()
                events = [
                    ("message_start", {"type": "message_start", "message": message}),
                    ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": block}),
                    ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(value)}}),
                    ("content_block_stop", {"type": "content_block_stop", "index": 0}),
                    ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": None}, "usage": {"output_tokens": 1}}),
                    ("message_stop", {"type": "message_stop"}),
                ]
                for event, data in events:
                    self.wfile.write(f"event: {event}\ndata: {json.dumps(data)}\n\n".encode())

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


@unittest.skipUnless(HAS_CLI, "Claude SDK and native CLI are required for runtime tests")
class ClaudeRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        from claude_agent_sdk._internal.transport.subprocess_cli import _ACTIVE_CHILDREN

        self.children = _ACTIVE_CHILDREN
        self.original_children = {child.pid for child in self.children}
        self.temp = tempfile.TemporaryDirectory(prefix="acr-claude-runtime-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        local_bin = self.root / "bin"
        local_bin.mkdir()
        (local_bin / "claude").symlink_to(CLI)
        (self.root / "hello.txt").write_text("hello from the repository\n")
        self.api = LocalAPI()
        self.addCleanup(self.api.close)
        self.env = patch.dict(os.environ, {
            "ANTHROPIC_API_KEY": "local-test-key",
            "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{self.api.server.server_port}",
            "CLAUDE_CONFIG_DIR": str(self.root / "config"),
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
            "PATH": str(local_bin) + os.pathsep + os.defpath,
        }, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)
        self.config = RunConfig(
            backend="claude-agent", review_model="sonnet", postprocess_model="sonnet",
            tools=ToolConfig(development=("source.read",)),
            timeout_seconds=15,
        )
        self.backend = create_backend(self.config, repo_root=self.root, run_root=self.root / "runs")
        self.addAsyncCleanup(self.backend.aclose)

    def assert_reaped(self):
        self.assertEqual({child.pid for child in self.children}, self.original_children)

    async def test_actual_cli_mcp_structured_output_and_configuration_isolation(self):
        # Exceed the CLI's normal inline tool-result threshold: native Read is
        # disabled, so a spooled result would make this evidence inaccessible.
        (self.root / "hello.txt").write_text("hello from the repository " + "x" * 35_000 + " INLINE_TOOL_END\n")
        marker = self.root / "unexpected-hook"
        hook = {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": f"touch {shlex.quote(str(marker))}"}]}]}}
        for directory in (self.root / ".claude", self.root / "config"):
            directory.mkdir()
            (directory / "settings.json").write_text(json.dumps(hook))
        (self.root / "CLAUDE.md").write_text("UNEXPECTED_PROJECT_MEMORY")
        (self.root / ".mcp.json").write_text(json.dumps({"mcpServers": {"outside": {"command": "should-never-execute"}}}))
        invocation = AgentFactory(self.config).persona(
            AgentInstructions(("development",), "Review using the ACR tools.", "test"),
            AgentInput.from_text("Read hello.txt and return no findings."), persona="development",
        )
        events = []
        gated = []
        original_gate = ClaudeTools.gate
        async def record_gate(tools, data, tool_id, context):
            gated.append(data.get("tool_name"))
            return await original_gate(tools, data, tool_id, context)
        with patch.object(ClaudeTools, "gate", record_gate):
            response = await self.backend.run(AgentRequest.from_invocation(invocation), event_sink=events.append)
        self.assertEqual(response.value, {"comments": []})
        self.assertEqual(response.usage["tool_calls"], 1)
        self.assertIn("hello from the repository", json.dumps(self.api.requests))
        self.assertIn("INLINE_TOOL_END", json.dumps(self.api.requests))
        self.assertNotIn("UNEXPECTED_PROJECT_MEMORY", json.dumps(self.api.requests))
        self.assertFalse(marker.exists())
        self.assertTrue(any(event.tool_name == "source.read" for event in events))
        self.assertIn("mcp__acr__source_read", gated)
        self.assert_reaped()

    async def test_actual_cli_inherits_user_settings_for_keys_bearer_tokens_and_helpers(self):
        config_dir = self.root / "config"
        config_dir.mkdir()
        # Leave no credentials in the caller environment: the native CLI must
        # resolve both routing and authentication from its own settings file.
        os.environ.pop("ANTHROPIC_API_KEY")
        os.environ.pop("ANTHROPIC_BASE_URL")
        config = replace(self.config, review_model="default", postprocess_model="default")
        backend = create_backend(config, repo_root=self.root, run_root=self.root / "runs")
        self.addAsyncCleanup(backend.aclose)
        invocation = AgentFactory(config).summary("Return a valid summary.")
        for name, header in (("ANTHROPIC_API_KEY", "x-api-key"), ("ANTHROPIC_AUTH_TOKEN", "authorization"), ("apiKeyHelper", "x-api-key")):
            with self.subTest(name=name):
                self.api.requests.clear()
                self.api.headers.clear()
                settings = {"model": "claude-local-test-model", "env": {
                    "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{self.api.server.server_port}",
                }}
                if name == "apiKeyHelper":
                    settings[name] = "printf local-dummy-credential"
                else:
                    settings["env"][name] = "local-dummy-credential"
                (config_dir / "settings.json").write_text(json.dumps(settings))
                events = []
                result = await backend.run(AgentRequest.from_invocation(invocation), event_sink=events.append)
                self.assertEqual(result.value, {"summary": "Local simulated review."})
                self.assertTrue(self.api.requests)
                self.assertTrue(all(body["model"] == "claude-local-test-model" for body in self.api.requests))
                self.assertTrue(all("local-dummy-credential" in {key.lower(): value for key, value in headers.items()}.get(header, "") for headers in self.api.headers))
                completed = next(event for event in events if event.event_type == "completed")
                self.assertEqual(completed.data["model"], "claude-local-test-model")
                self.assert_reaped()

    async def test_unchanged_profile_runs_the_existing_workflow_with_local_cli_settings(self):
        config_dir = self.root / "config"
        config_dir.mkdir()
        (config_dir / "settings.json").write_text(json.dumps({
            "model": "claude-user-test-model", "env": {
                "ANTHROPIC_API_KEY": "local-workflow-key",
                "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{self.api.server.server_port}",
            },
        }))
        project_config = self.root / ".claude"
        project_config.mkdir()
        (project_config / "settings.json").write_text(json.dumps({"model": "claude-project-test-model"}))
        (project_config / "settings.local.json").write_text(json.dumps({"model": "claude-workflow-test-model"}))
        os.environ.pop("ANTHROPIC_API_KEY")
        os.environ.pop("ANTHROPIC_BASE_URL")
        source_root = Path(__file__).resolve().parents[1]
        os.environ["ACR_GUIDELINE_ROOT"] = str(source_root.parents[1])
        for command in (
            ["git", "init", "-q"],
            ["git", "add", "hello.txt"],
            ["git", "-c", "user.name=ACR Test", "-c", "user.email=acr@example.invalid", "commit", "-qm", "test fixture"],
        ):
            subprocess.run(command, cwd=self.root, check=True, capture_output=True)
        config = replace(load_config(source_root / "acr.claude.toml"), log_root=self.root / "logs")
        output = self.root / "review.md"
        await asyncio.wait_for(run_review(f"files hello.txt {output}", config, repo_root=self.root), 30)
        self.assertIn("Local simulated review.", output.read_text())
        self.assertTrue(all(body["model"] == "claude-workflow-test-model" for body in self.api.requests))
        self.assert_reaped()

    async def test_actual_cli_accepts_all_postprocessing_schemas(self):
        for role in ("verification", "consolidation", "summary", "grader"):
            with self.subTest(role=role):
                invocation = getattr(AgentFactory(self.config), role)("Return a valid result.")
                response = await self.backend.run(AgentRequest.from_invocation(invocation))
                invocation.output_type.model_validate(response.value)
                self.assert_reaped()

    async def test_actual_cli_reaps_child_on_timeout_token_task_and_backend_close(self):
        self.api.hang = True
        for mode in ("timeout", "token", "task", "close"):
            with self.subTest(mode=mode):
                self.api.started.clear()
                backend = create_backend(self.config, repo_root=self.root, run_root=self.root / "runs")
                token = CancellationToken()
                request = AgentRequest("summary", "Return the result.", "wait", CommentsEnvelope, model="sonnet", timeout_seconds=2 if mode == "timeout" else 10)
                task = asyncio.create_task(backend.run(request, cancellation=token))
                try:
                    self.assertTrue(await asyncio.to_thread(self.api.started.wait, 5))
                    if mode == "token":
                        token.cancel()
                    elif mode == "task":
                        task.cancel()
                        await asyncio.sleep(0.01)
                        task.cancel()  # Repeated caller cancellation must not interrupt reaping.
                    elif mode == "close":
                        await backend.aclose()
                    with self.assertRaises(ClaudeAgentError if mode == "timeout" else asyncio.CancelledError):
                        await asyncio.wait_for(task, 15)
                    self.assert_reaped()
                finally:
                    await backend.aclose()
