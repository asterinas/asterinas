"""Read-only repository tools exposed through :class:`ToolBroker`."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..agents.protocol import ToolSpec
from .protocol import Tool, ToolResult


MAX_READ_LINES = 180
MAX_SEARCH_QUERY_CHARS = 500
MAX_SEARCH_MATCHES = 200
MAX_LIST_FILES = 50


def _safe_path(root: Path, value: str) -> Path:
    candidate = (root / value).resolve()
    if candidate != root and root not in candidate.parents:
        raise ValueError("path escapes repository root")
    return candidate


class SourceReadTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "source.read",
            (
                "Read an inclusive, 1-based line range from one UTF-8 text file in the "
                f"repository. A call may request at most {MAX_READ_LINES} lines. If end is "
                "past EOF, the result stops at EOF. Each returned line is prefixed with its "
                "line number."
            ),
            {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Repository-relative path to a UTF-8 text file.",
                    },
                    "start": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "First line to return (1-based, inclusive).",
                    },
                    "end": {
                        "type": "integer",
                        "minimum": 1,
                        "description": (
                            "Last line to return (1-based, inclusive). Must be greater than or "
                            f"equal to start, with end - start + 1 <= {MAX_READ_LINES}."
                        ),
                    },
                },
                "required": ["path", "start", "end"],
                "additionalProperties": False,
            },
        )

    async def run(self, arguments: dict[str, Any]) -> ToolResult:
        try:
            path = _safe_path(self.repo_root, str(arguments["path"]))
            start, end = int(arguments["start"]), int(arguments["end"])
            if start < 1 or end < start or end - start + 1 > MAX_READ_LINES:
                return ToolResult(
                    False,
                    error_code="TOOL_INVALID_INPUT",
                    value=(
                        "start and end must define an inclusive, 1-based range of at most "
                        f"{MAX_READ_LINES} lines"
                    ),
                )
            if not path.is_file():
                return ToolResult(False, error_code="SOURCE_ERROR", value="path is not a file")
            lines = path.read_text(encoding="utf-8").splitlines()
            return ToolResult(True, "\n".join(f"{i}: {lines[i - 1]}" for i in range(start, min(end, len(lines)) + 1)))
        except (KeyError, ValueError, OSError, UnicodeDecodeError) as exc:
            return ToolResult(False, error_code="SOURCE_ERROR", value=str(exc))


class SourceSearchTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "source.search",
            (
                "Recursively search UTF-8 repository files for a literal, case-sensitive "
                f"substring. Returns at most {MAX_SEARCH_MATCHES} matches as path:line:text; "
                "files are searched in lexical order, and a result at that limit may be "
                "truncated. Narrow path or query to continue. Unreadable or non-UTF-8 files "
                "and .git, target, and __pycache__ trees are skipped."
            ),
            {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_SEARCH_QUERY_CHARS,
                        "description": "Literal substring to find; regular expressions are not supported.",
                    },
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": (
                            'Repository-relative directory to search recursively. Use "." for the '
                            "repository root."
                        ),
                    },
                },
                "required": ["query", "path"],
                "additionalProperties": False,
            },
        )

    async def run(self, arguments: dict[str, Any]) -> ToolResult:
        try:
            query = str(arguments["query"])
            if not query or len(query) > MAX_SEARCH_QUERY_CHARS:
                return ToolResult(
                    False,
                    error_code="TOOL_INVALID_INPUT",
                    value=f"query must contain 1 to {MAX_SEARCH_QUERY_CHARS} characters",
                )
            root = _safe_path(self.repo_root, str(arguments["path"]))
            if not root.is_dir():
                return ToolResult(False, error_code="SOURCE_ERROR", value="path is not a directory")
            matches: list[str] = []
            for path in sorted(root.rglob("*")):
                if not path.is_file() or any(part in {".git", "target", "__pycache__"} for part in path.parts):
                    continue
                try:
                    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                        if query in line:
                            matches.append(f"{path.relative_to(self.repo_root)}:{number}:{line}")
                            if len(matches) >= MAX_SEARCH_MATCHES:
                                return ToolResult(True, "\n".join(matches))
                except (OSError, UnicodeDecodeError):
                    continue
            return ToolResult(True, "\n".join(matches))
        except (KeyError, ValueError, OSError, UnicodeDecodeError) as exc:
            return ToolResult(False, error_code="SOURCE_ERROR", value=str(exc))


class SourceListTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "source.list",
            (
                "Recursively list repository files below a directory. Returns repository-relative "
                f"paths in lexical order for at most {MAX_LIST_FILES} files; a result at that "
                "limit may be truncated. Narrow path to inspect a truncated subtree."
            ),
            {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": (
                            'Repository-relative directory to list recursively. Use "." for the '
                            "repository root."
                        ),
                    }
                },
                "required": ["path"],
                "additionalProperties": False,
            },
        )

    async def run(self, arguments: dict[str, Any]) -> ToolResult:
        try:
            root = _safe_path(self.repo_root, str(arguments["path"]))
            if not root.is_dir():
                return ToolResult(False, error_code="SOURCE_ERROR", value="path is not a directory")
            values = sorted(str(path.relative_to(self.repo_root)) for path in root.rglob("*") if path.is_file())
            return ToolResult(True, "\n".join(values[:MAX_LIST_FILES]))
        except (KeyError, ValueError, OSError, UnicodeDecodeError) as exc:
            return ToolResult(False, error_code="SOURCE_ERROR", value=str(exc))


class SourceToolProvider:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root.resolve()
        tools: tuple[Tool, ...] = (
            SourceReadTool(self.repo_root),
            SourceSearchTool(self.repo_root),
            SourceListTool(self.repo_root),
        )
        self._tools = {tool.spec().name: tool for tool in tools}

    def specs(self) -> tuple[ToolSpec, ...]:
        return tuple(tool.spec() for tool in self._tools.values())

    async def invoke(self, name: str, arguments: dict[str, Any]) -> ToolResult:
        tool = self._tools.get(name)
        if tool is None:
            return ToolResult(False, error_code="TOOL_DENIED")
        return await tool.run(arguments)
