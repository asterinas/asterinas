"""Read-only repository tools exposed through :class:`ToolBroker`."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..agents.protocol import ToolSpec
from .protocol import Tool, ToolResult


MAX_READ_LINES = 180
MAX_READ_RANGES = 8
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
                f"Read 1 to {MAX_READ_RANGES} inclusive, 1-based ranges from repository UTF-8 "
                f"files. Each range returns at most {MAX_READ_LINES} numbered lines, the actual "
                "returned_range (null if empty), total_lines, eof, and next_start. If next_start "
                "is non-null, repeat that range with start=next_start and the original end. "
                "Results preserve request order and include the original request; errors are "
                "reported per range so other reads can succeed. Invalid bounds include a retry range."
            ),
            {
                "type": "object",
                "properties": {
                    "ranges": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_READ_RANGES,
                        "description": "File ranges to read; use one item for a single read.",
                        "items": {
                            "type": "object",
                            "description": "One file and its requested inclusive line range.",
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
                                    "description": "Last requested line, inclusive and >= start; long ranges are paged.",
                                },
                            },
                            "required": ["path", "start", "end"],
                            "additionalProperties": False,
                        },
                    }
                },
                "required": ["ranges"],
                "additionalProperties": False,
            },
        )

    async def run(self, arguments: dict[str, Any]) -> ToolResult:
        ranges = arguments.get("ranges")
        if not isinstance(ranges, list) or not 1 <= len(ranges) <= MAX_READ_RANGES:
            return ToolResult(
                False,
                error_code="TOOL_INVALID_INPUT",
                value=f"provide ranges as an array of 1 to {MAX_READ_RANGES} objects with path, start and end",
            )
        return ToolResult(True, [{"request": request, **self._read_range(request)} for request in ranges])

    def _read_range(self, request: Any) -> dict[str, Any]:
        if (
            not isinstance(request, dict)
            or not isinstance(request.get("path"), str)
            or not request["path"]
            or type(request.get("start")) is not int
            or type(request.get("end")) is not int
        ):
            return {"error_code": "TOOL_INVALID_INPUT", "error": "provide a nonempty path and integer start/end"}
        start, end = request["start"], request["end"]
        if start < 1 or end < start:
            return {
                "error_code": "TOOL_INVALID_INPUT",
                "error": "use inclusive bounds with 1 <= start <= end",
                "retry": {"path": request["path"], "start": max(1, start), "end": max(1, start, end)},
            }
        try:
            path = _safe_path(self.repo_root, request["path"])
            if not path.is_file():
                return {"error_code": "SOURCE_ERROR", "error": "path is not a file; check the repository-relative path"}
            lines = path.read_text(encoding="utf-8").splitlines()
            stop = min(end, start + MAX_READ_LINES - 1, len(lines))
            return {
                "content": "\n".join(f"{i}: {lines[i - 1]}" for i in range(start, stop + 1)),
                "returned_range": [start, stop] if start <= stop else None,
                "total_lines": len(lines),
                "eof": stop == len(lines),
                "next_start": stop + 1 if stop < min(end, len(lines)) else None,
            }
        except (ValueError, OSError) as exc:
            return {"error_code": "SOURCE_ERROR", "error": str(exc)}


class SourceSearchTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "source.search",
            (
                "Search one UTF-8 repository file or a directory recursively for a literal, case-sensitive "
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
                            'Repository-relative file or directory to search. Use "." for the '
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
            if not root.is_file() and not root.is_dir():
                return ToolResult(False, error_code="SOURCE_ERROR", value="path is not a file or directory")
            matches: list[str] = []
            for path in [root] if root.is_file() else sorted(root.rglob("*")):
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
