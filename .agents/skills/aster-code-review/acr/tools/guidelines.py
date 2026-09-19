"""Tool-provider facade for progressive guideline queries."""

from __future__ import annotations

from pathlib import Path

from ..agents.protocol import ToolSpec
from ..core.contracts import PERSONAS
from ..core.disclosure import query_exact
from .protocol import Tool, ToolResult


class GuidelineShowTool:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def spec(self) -> ToolSpec:
        return ToolSpec(
            "guideline.show",
            (
                "Fetch exact authored sections for one or more suspected guideline violations. "
                "Use the persona, digest, and short-names exactly as shown in that persona's "
                "GUIDELINE_CATALOG. A stale digest or a short-name outside that catalog fails the "
                "entire call. Returned sections follow catalog order, not request order."
            ),
            {
                "type": "object",
                "properties": {
                    "persona": {
                        "type": "string",
                        "enum": list(PERSONAS),
                        "description": "Owning persona named in the relevant GUIDELINE_CATALOG header.",
                    },
                    "digest": {
                        "type": "string",
                        "pattern": "^[0-9a-f]{64}$",
                        "description": "64-character digest copied from that catalog header.",
                    },
                    "short_names": {
                        "type": "array",
                        "minItems": 1,
                        "description": (
                            "One or more candidate rule short-names from that persona's catalog; "
                            "batch related candidates in one call."
                        ),
                        "items": {
                            "type": "string",
                            "pattern": "^[a-z0-9][a-z0-9-]*$",
                            "description": "A guideline short-name exactly as listed in the catalog.",
                        },
                    },
                },
                "required": ["persona", "digest", "short_names"],
                "additionalProperties": False,
            },
            "guideline",
        )

    async def run(self, arguments: dict[str, object]) -> ToolResult:
        try:
            value = query_exact(self.repo_root, str(arguments["persona"]), str(arguments["digest"]), [str(item) for item in arguments.get("short_names", [])])
            return ToolResult(True, value)
        except Exception as exc:
            return ToolResult(False, error_code="GUIDELINE_QUERY_ERROR", value=str(exc))


class GuidelineToolProvider:
    def __init__(self, repo_root: Path):
        self.repo_root = repo_root
        tools: tuple[Tool, ...] = (GuidelineShowTool(self.repo_root),)
        self._tools = {tool.spec().name: tool for tool in tools}

    def specs(self) -> tuple[ToolSpec, ...]:
        return tuple(tool.spec() for tool in self._tools.values())

    async def invoke(self, name: str, arguments: dict[str, object]) -> ToolResult:
        tool = self._tools.get(name)
        if tool is None:
            return ToolResult(False, error_code="TOOL_DENIED")
        return await tool.run(arguments)
