"""Git/worktree interfaces used by stages and benchmark control plane."""

from __future__ import annotations

import asyncio
import subprocess
from pathlib import Path

from ..agents.protocol import ToolSpec
from .protocol import Tool, ToolResult


_FORBIDDEN_ARGUMENTS = ("--output", "--ext-diff", "--no-ext-diff", "--exec-path", "--config", "-c")


def _has_forbidden_argument(arguments: list[str]) -> bool:
    return any(item == flag or item.startswith(flag + "=") for item in arguments for flag in _FORBIDDEN_ARGUMENTS)


async def _run_git(repo_root: Path, command: list[str]) -> ToolResult:
    completed = await asyncio.to_thread(subprocess.run, ["git", *command], cwd=repo_root, capture_output=True, text=True, check=False)
    if completed.returncode:
        return ToolResult(False, error_code="GIT_ERROR", value=completed.stderr.strip())
    return ToolResult(True, completed.stdout)


class GitShowTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "git.show",
            (
                "Run git show for one revision or object expression and return its text output. "
                "Pass exactly one expression, such as HEAD, <commit>, or <revision>:<path>; do "
                "not combine multiple command-line arguments in this string."
            ),
            {
                "type": "object",
                "properties": {
                    "revision": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Single revision or object expression accepted by git show.",
                    }
                },
                "required": ["revision"],
                "additionalProperties": False,
            },
            "git",
        )

    async def run(self, arguments: dict[str, object]) -> ToolResult:
        supplied = [str(arguments.get("revision", "HEAD"))]
        if _has_forbidden_argument(supplied):
            return ToolResult(False, error_code="TOOL_DENIED")
        return await _run_git(self.repo_root, ["show", str(arguments.get("revision", "HEAD"))])


class GitDiffTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "git.diff",
            (
                "Run read-only git diff and return its text output. Each array item is passed as "
                "one separate argument after `git diff`; use an empty array for the unstaged "
                "worktree diff. Scope large results with revisions, pathspecs, or diff options. "
                "Options that write output, control external diffs, or change Git configuration are denied."
            ),
            {
                "type": "object",
                "properties": {
                    "arguments": {
                        "type": "array",
                        "description": (
                            "Ordered git diff arguments, one command-line argument per item; for "
                            "example [\"HEAD~1\", \"HEAD\", \"--\", \"kernel/src/foo.rs\"]."
                        ),
                        "items": {
                            "type": "string",
                            "description": "One distinct command-line argument passed to git diff.",
                        },
                    }
                },
                "required": ["arguments"],
                "additionalProperties": False,
            },
            "git",
        )

    async def run(self, arguments: dict[str, object]) -> ToolResult:
        supplied = [str(item) for item in arguments.get("arguments", [])]
        if _has_forbidden_argument(supplied):
            return ToolResult(False, error_code="TOOL_DENIED")
        return await _run_git(self.repo_root, ["diff", *supplied])


class GitLogTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "git.log",
            (
                "Run read-only git log and return its text output. Each array item is passed as "
                "one separate argument after `git log`; use an empty array for the default history. "
                "Use options such as --max-count=<n>, revisions, or pathspecs to bound large results. "
                "Options that write output, control external diffs, or change Git configuration are denied."
            ),
            {
                "type": "object",
                "properties": {
                    "arguments": {
                        "type": "array",
                        "description": (
                            "Ordered git log arguments, one command-line argument per item; for "
                            "example [\"--max-count=20\", \"--oneline\", \"HEAD\"]."
                        ),
                        "items": {
                            "type": "string",
                            "description": "One distinct command-line argument passed to git log.",
                        },
                    }
                },
                "required": ["arguments"],
                "additionalProperties": False,
            },
            "git",
        )

    async def run(self, arguments: dict[str, object]) -> ToolResult:
        supplied = [str(item) for item in arguments.get("arguments", [])]
        if _has_forbidden_argument(supplied):
            return ToolResult(False, error_code="TOOL_DENIED")
        return await _run_git(self.repo_root, ["log", *supplied])


class GitBlameTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "git.blame",
            "Run git blame for one repository file and return attribution for all of its lines.",
            {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Repository-relative file path. Command-line options are not accepted.",
                    }
                },
                "required": ["path"],
                "additionalProperties": False,
            },
            "git",
        )

    async def run(self, arguments: dict[str, object]) -> ToolResult:
        value = str(arguments.get("path", ""))
        if Path(value).is_absolute():
            return ToolResult(
                False,
                error_code="TOOL_DENIED",
                value="path must be repository-relative",
            )
        candidate = (self.repo_root / value).resolve()
        if not value or (candidate != self.repo_root and self.repo_root not in candidate.parents):
            return ToolResult(
                False,
                error_code="TOOL_DENIED",
                value="path must stay within the repository",
            )
        path = candidate.relative_to(self.repo_root).as_posix()
        return await _run_git(self.repo_root, ["blame", "--", path])


class GitToolProvider:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root.resolve()
        tools: tuple[Tool, ...] = (
            GitShowTool(self.repo_root),
            GitDiffTool(self.repo_root),
            GitLogTool(self.repo_root),
            GitBlameTool(self.repo_root),
        )
        self._tools = {tool.spec().name: tool for tool in tools}
        self._aliases = {
            "git_show": "git.show",
            "git_diff": "git.diff",
            "git_log": "git.log",
        }

    def specs(self) -> tuple[ToolSpec, ...]:
        return tuple(tool.spec() for tool in self._tools.values())

    async def invoke(self, name: str, arguments: dict[str, object]) -> ToolResult:
        tool = self._tools.get(self._aliases.get(name, name))
        if tool is None:
            return ToolResult(False, error_code="TOOL_DENIED")
        return await tool.run(arguments)
