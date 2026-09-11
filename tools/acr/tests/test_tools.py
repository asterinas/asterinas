from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

from acr.agents.openai_agents.tools import function_tools
from acr.core.contracts import PERSONAS
from acr.tools.builtin import default_provider
from acr.tools.git import GitBlameTool, GitDiffTool, GitLogTool, GitShowTool, GitToolProvider
from acr.tools.guidelines import GuidelineShowTool
from acr.tools.protocol import ToolBroker
from acr.tools.repository import (
    MAX_LIST_FILES,
    MAX_READ_LINES,
    MAX_READ_RANGES,
    MAX_SEARCH_MATCHES,
    MAX_SEARCH_QUERY_CHARS,
    SourceListTool,
    SourceReadTool,
    SourceSearchTool,
)


class ToolSpecTests(unittest.TestCase):
    def test_every_builtin_argument_has_a_description(self):
        root = Path(".").resolve()
        tools = (
            SourceReadTool(root),
            SourceSearchTool(root),
            SourceListTool(root),
            GitShowTool(root),
            GitDiffTool(root),
            GitLogTool(root),
            GitBlameTool(root),
            GuidelineShowTool(root),
        )

        for tool in tools:
            spec = tool.spec()
            self.assertTrue(spec.description, spec.name)
            self.assertEqual(
                set(spec.input_schema["required"]),
                set(spec.input_schema["properties"]),
                spec.name,
            )
            for name, schema in spec.input_schema["properties"].items():
                self.assertTrue(schema.get("description"), f"{spec.name}.{name}")
                if schema.get("type") == "array":
                    self.assertTrue(
                        schema["items"].get("description"),
                        f"{spec.name}.{name}[]",
                    )

    def test_repository_specs_expose_their_exact_limits(self):
        root = Path(".").resolve()
        read = SourceReadTool(root).spec()
        search = SourceSearchTool(root).spec()
        listing = SourceListTool(root).spec()

        self.assertIn(str(MAX_READ_LINES), read.description)
        self.assertEqual(search.input_schema["properties"]["query"]["maxLength"], MAX_SEARCH_QUERY_CHARS)
        self.assertEqual(search.input_schema["required"], ["query", "path"])
        self.assertIn(str(MAX_SEARCH_MATCHES), search.description)
        self.assertEqual(listing.input_schema["required"], ["path"])
        self.assertIn(str(MAX_LIST_FILES), listing.description)

    def test_guideline_spec_constrains_catalog_identifiers(self):
        spec = GuidelineShowTool(Path(".").resolve()).spec()
        properties = spec.input_schema["properties"]

        self.assertEqual(tuple(properties["persona"]["enum"]), PERSONAS)
        self.assertEqual(properties["digest"]["pattern"], "^[0-9a-f]{64}$")
        self.assertEqual(properties["short_names"]["minItems"], 1)

    def test_sdk_descriptions_expose_broker_execution_limits(self):
        broker = ToolBroker(
            default_provider(Path(".").resolve()),
            timeout_seconds=7.5,
            max_output_bytes=1234,
        )

        for tool in function_tools(broker):
            self.assertIn("7.5 seconds", tool.description, tool.name)
            self.assertIn("1234 bytes", tool.description, tool.name)


class GitToolTests(unittest.IsolatedAsyncioTestCase):
    """Regression coverage for the Git tool timeout in PR #3834."""

    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.enterContext(patch.dict(os.environ, {"PATH": str(self.root)}))

    def install_git(self, program: str) -> None:
        executable = self.root / "git"
        executable.write_text(f"#!{sys.executable}\n" + textwrap.dedent(program), encoding="utf-8")
        executable.chmod(0o700)

    def install_slow_git(self) -> Path:
        self.install_git("""
            import os
            import signal
            import time
            from pathlib import Path

            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            Path("pid").write_text(str(os.getpid()))
            time.sleep(5)
        """)
        return self.root / "pid"

    async def test_git_preserves_success_and_error_output(self):
        self.install_git(r"""
            import sys

            if sys.argv[1] == "log":
                sys.stdout.buffer.write(b"history\r\n")
            else:
                sys.stderr.write("bad revision\n")
                sys.exit(1)
        """)
        broker = ToolBroker(GitToolProvider(self.root))
        success = await broker.invoke("git.log", {"arguments": []})
        self.assertTrue(success.ok)
        self.assertEqual(success.value, "history\n")
        failure = await broker.invoke("git.show", {"revision": "missing"})
        self.assertFalse(failure.ok)
        self.assertEqual(failure.error_code, "GIT_ERROR")
        self.assertEqual(failure.value, "bad revision")

    async def test_git_timeout_reaps_a_child_that_ignores_termination(self):
        pid_file = self.install_slow_git()
        events = []
        broker = ToolBroker(GitToolProvider(self.root), timeout_seconds=1, event_hook=events.append)
        with self.assertRaises(TimeoutError):
            await broker.invoke("git.log", {"arguments": []})
        self.assertEqual(events[-1]["error_code"], "TOOL_TIMEOUT")
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)

    async def test_git_repeated_cancellation_waits_for_child_cleanup(self):
        pid_file = self.install_slow_git()
        broker = ToolBroker(GitToolProvider(self.root))
        call = asyncio.create_task(broker.invoke("git.log", {"arguments": []}))
        try:
            async with asyncio.timeout(2):
                while not pid_file.exists():
                    await asyncio.sleep(0.01)
        finally:
            call.cancel()
        await asyncio.sleep(0.01)
        call.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await call
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)


class RepositoryToolTests(unittest.TestCase):
    def test_source_read_pages_without_gaps_or_duplicates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            total = MAX_READ_LINES * 2 + 1
            (root / "lines.txt").write_text(
                "".join(f"line {number}\n" for number in range(1, total + 1)),
                encoding="utf-8",
            )
            tool = SourceReadTool(root)
            request = {"path": "lines.txt", "start": 1, "end": total + 100}
            content = []
            for start in (1, MAX_READ_LINES + 1, MAX_READ_LINES * 2 + 1):
                self.assertEqual(request["start"], start)
                result = asyncio.run(tool.run({"ranges": [request]}))
                self.assertTrue(result.ok)
                page = result.value[0]
                self.assertEqual(page["request"], request)
                self.assertEqual(page["total_lines"], total)
                self.assertEqual(page["returned_range"], [start, min(start + MAX_READ_LINES - 1, total)])
                content.extend(page["content"].splitlines())
                request = {**request, "start": page["next_start"]}
            self.assertIsNone(page["next_start"])
            self.assertTrue(page["eof"])
            self.assertEqual(content, [f"{i}: line {i}" for i in range(1, total + 1)])

    def test_source_read_stops_at_requested_end_or_eof(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lines.txt").write_text("first\nsecond\n", encoding="utf-8")
            (root / "empty.txt").write_text("", encoding="utf-8")
            tool = SourceReadTool(root)
            for path, start, end, expected_range, total, eof in (
                ("lines.txt", 1, 1, [1, 1], 2, False),
                ("lines.txt", 2, 1000, [2, 2], 2, True),
                ("lines.txt", 3, 1000, None, 2, True),
                ("empty.txt", 1, 1000, None, 0, True),
            ):
                with self.subTest(path=path, start=start, end=end):
                    result = asyncio.run(tool.run({"ranges": [{"path": path, "start": start, "end": end}]}))
                    page = result.value[0]
                    self.assertEqual(page["returned_range"], expected_range)
                    self.assertEqual(page["total_lines"], total)
                    self.assertEqual(page["eof"], eof)
                    self.assertIsNone(page["next_start"])
                    if expected_range is None:
                        self.assertEqual(page["content"], "")

    def test_source_read_batch_keeps_successes_and_returns_retry_through_sdk(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a.txt").write_text("alpha\n", encoding="utf-8")
            (root / "b.txt").write_text("beta\n", encoding="utf-8")
            broker = ToolBroker(default_provider(root), allowed={"source.read"})
            tool = function_tools(broker)[0]
            requests = [
                {"path": "a.txt", "start": 1, "end": 10},
                {"path": "missing.txt", "start": 1, "end": 10},
                {"path": "b.txt", "start": 0, "end": 1},
                {"path": "b.txt", "start": 1, "end": 10},
            ]
            response = asyncio.run(tool.on_invoke_tool(None, json.dumps({"ranges": requests})))
            pages = json.loads(response)
            self.assertEqual([page["request"] for page in pages], requests)
            self.assertEqual(pages[0]["content"], "1: alpha")
            self.assertEqual(pages[1]["error_code"], "SOURCE_ERROR")
            self.assertEqual(pages[2]["error_code"], "TOOL_INVALID_INPUT")
            self.assertEqual(pages[3]["content"], "1: beta")
            retry = asyncio.run(broker.invoke("source.read", {"ranges": [pages[2]["retry"]]}))
            self.assertEqual(retry.value[0]["content"], "1: beta")

    def test_source_read_rejects_invalid_batches_and_range_types(self):
        tool = SourceReadTool(Path(".").resolve())
        for ranges in (None, {}, [], [None] * (MAX_READ_RANGES + 1)):
            with self.subTest(ranges=ranges):
                result = asyncio.run(tool.run({"ranges": ranges}))
                self.assertFalse(result.ok)
                self.assertEqual(result.error_code, "TOOL_INVALID_INPUT")
        result = asyncio.run(tool.run({"ranges": [
            None,
            {"path": "a.txt", "start": True, "end": 2},
            {"path": "a.txt", "start": 1.5, "end": 2},
        ]}))
        self.assertEqual([page["error_code"] for page in result.value], ["TOOL_INVALID_INPUT"] * 3)

    def test_source_search_accepts_a_file_or_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a.txt").write_text("needle\nNeedle\nneedle.*\n", encoding="utf-8")
            (root / "b.txt").write_text("needle\n", encoding="utf-8")
            tool = SourceSearchTool(root)
            for path, query, expected in (
                ("a.txt", "needle", "a.txt:1:needle\na.txt:3:needle.*"),
                (".", "needle", "a.txt:1:needle\na.txt:3:needle.*\nb.txt:1:needle"),
                ("a.txt", "needle.*", "a.txt:3:needle.*"),
                ("a.txt", "absent", ""),
            ):
                with self.subTest(path=path, query=query):
                    result = asyncio.run(tool.run({"path": path, "query": query}))
                    self.assertTrue(result.ok)
                    self.assertEqual(result.value, expected)
            missing = asyncio.run(tool.run({"path": "missing.txt", "query": "needle"}))
            self.assertFalse(missing.ok)

    def test_source_search_filters_only_repository_relative_paths(self):
        """Regression coverage for path filtering in PR #3834."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "target" / "repo"
            (root / "target").mkdir(parents=True)
            (root / "README.md").write_text("needle\n", encoding="utf-8")
            (root / "target" / "artifact.txt").write_text("needle\n", encoding="utf-8")
            tool = SourceSearchTool(root)

            for path in (".", "README.md"):
                with self.subTest(path=path):
                    result = asyncio.run(tool.run({"path": path, "query": "needle"}))
                    self.assertTrue(result.ok)
                    self.assertEqual(result.value, "README.md:1:needle")

    def test_source_search_validates_recursively_discovered_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            root.mkdir()
            outside = Path(directory) / "outside.txt"
            outside.write_text("needle private", encoding="utf-8")
            (root / "a-external.txt").symlink_to(outside)
            (root / "b-internal.txt").symlink_to("local.txt")
            (root / "local.txt").write_text("needle local", encoding="utf-8")

            result = asyncio.run(SourceSearchTool(root).run({"path": ".", "query": "needle"}))

            self.assertTrue(result.ok)
            self.assertEqual(result.value, "b-internal.txt:1:needle local\nlocal.txt:1:needle local")

    def test_read_and_file_search_reject_paths_outside_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            root.mkdir()
            outside = Path(directory) / "outside.txt"
            outside.write_text("private", encoding="utf-8")
            (root / "link.txt").symlink_to(outside)
            for path in ("../outside.txt", "link.txt"):
                with self.subTest(path=path):
                    read = asyncio.run(SourceReadTool(root).run({"ranges": [{"path": path, "start": 1, "end": 2}]}))
                    self.assertEqual(read.value[0]["error_code"], "SOURCE_ERROR")
                    self.assertNotIn("content", read.value[0])
                    search = asyncio.run(SourceSearchTool(root).run({"path": path, "query": "private"}))
                    self.assertFalse(search.ok)
