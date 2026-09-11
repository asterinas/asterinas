from __future__ import annotations

import asyncio
import tempfile
import unittest
from pathlib import Path

from acr.core.contracts import PERSONAS
from acr.tools.builtin import default_provider
from acr.tools.git import GitBlameTool, GitDiffTool, GitLogTool, GitShowTool
from acr.tools.guidelines import GuidelineShowTool
from acr.tools.protocol import ToolBroker
from acr.tools.repository import (
    MAX_LIST_FILES,
    MAX_READ_LINES,
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

        for tool in broker.function_tools():
            self.assertIn("7.5 seconds", tool.description, tool.name)
            self.assertIn("1234 bytes", tool.description, tool.name)


class RepositoryToolTests(unittest.TestCase):
    def test_source_read_accepts_exact_limit_and_rejects_one_more_line(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lines.txt").write_text(
                "".join(f"line {number}\n" for number in range(1, MAX_READ_LINES + 2)),
                encoding="utf-8",
            )
            tool = SourceReadTool(root)

            accepted = asyncio.run(
                tool.run({"path": "lines.txt", "start": 1, "end": MAX_READ_LINES})
            )
            rejected = asyncio.run(
                tool.run({"path": "lines.txt", "start": 1, "end": MAX_READ_LINES + 1})
            )

        self.assertTrue(accepted.ok)
        self.assertEqual(len(accepted.value.splitlines()), MAX_READ_LINES)
        self.assertFalse(rejected.ok)
        self.assertEqual(rejected.error_code, "TOOL_INVALID_INPUT")
