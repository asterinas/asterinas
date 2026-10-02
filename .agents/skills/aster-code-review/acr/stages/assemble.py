"""Deterministic fragment validation, sorting, deduplication, and rendering."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Mapping, Sequence

from ..core.contracts import PERSONAS, ReviewComment, validate_fragment
from ..core.state import RunContext


class AssemblyError(RuntimeError):
    code = "ASSEMBLY_ERROR"


@dataclass
class ReviewDocument:
    meta: dict[str, str]
    comments: list[ReviewComment]
    retracted: list[tuple[str, str]] = field(default_factory=list)
    summary: str | None = None

    def as_json(self) -> dict[str, object]:
        return {
            "meta": self.meta,
            "comments": [comment.model_dump(exclude_none=True) for comment in self.comments],
            "retracted": [{"comment_id": cid, "reason": reason} for cid, reason in self.retracted],
        }


_KEBAB = re.compile(r"[a-z0-9]+(-[a-z0-9]+)*")
_ORDER = (("maintainability", "Maintainability"), ("development", "Correctness"), ("security", "Security"), ("hardware", "Hardware"), ("documentation", "Documentation"))


def grounding_tag(value: str) -> str:
    return f"`{value}`" if _KEBAB.fullmatch(value) else value


def _sort_key(comment: ReviewComment):
    return (comment.file, comment.line or 0, comment.grounding, comment.problem)


def validate_fragments(fragments: Mapping[str, object], activated: Sequence[str]) -> dict[str, list[ReviewComment]]:
    result: dict[str, list[ReviewComment]] = {}
    for persona in activated:
        if persona not in fragments:
            raise AssemblyError(f"missing fragment for activated persona: {persona}")
        try:
            result[persona] = validate_fragment(fragments[persona])
        except Exception as exc:
            raise AssemblyError(f"invalid fragment for {persona}: {exc}") from exc
    return result


def assemble_fragments(meta: Mapping[str, str], fragments: Mapping[str, object], activated: Sequence[str], *, context: RunContext | None = None) -> ReviewDocument:
    validated = validate_fragments(fragments, activated)
    comments: list[ReviewComment] = []
    for persona, _title in _ORDER:
        if persona not in validated:
            continue
        seen: set[str] = set()
        unique: list[ReviewComment] = []
        for comment in validated[persona]:
            key = json.dumps(comment.model_dump(exclude_none=True), sort_keys=True, ensure_ascii=False)
            if key in seen:
                continue
            seen.add(key)
            unique.append(comment)
        comments.extend(sorted(unique, key=_sort_key))
    document = ReviewDocument(dict(meta), comments)
    if context is not None:
        context.write_json("artifacts/assembled.json", document.as_json())
    return document


def render_markdown(document: ReviewDocument, *, summary: str | None = None) -> str:
    meta = document.meta
    lines = ["---", f"date: {meta.get('date', '')}", f"mode: {meta.get('mode', 'diff')}"]
    if meta.get("base"):
        lines.append(f"base: {meta['base']}")
    if meta.get("files"):
        lines.append(f"files: {meta['files']}")
    lines.extend([f"head: {meta.get('head', '')}", f"branch: {meta.get('branch', '')}"])
    if meta.get("title"):
        lines.append("title: " + json.dumps(meta["title"], ensure_ascii=False))
    lines.extend(["---", "", "# Summary", "", summary if summary is not None else "<!-- SUMMARY -->", ""])

    by_persona: dict[str, list[ReviewComment]] = {persona: [] for persona, _ in _ORDER}
    for comment in document.comments:
        by_persona.setdefault(comment.persona, []).append(comment)
    for persona, title in _ORDER:
        comments = by_persona.get(persona, [])
        if not comments:
            continue
        lines.extend([f"## {title}", ""])
        for comment in comments:
            location = f"`{comment.file}`" + (f" line {comment.line}" if comment.line is not None else "")
            lines.extend([f"### {location}", ""])
            if comment.diff:
                lines.append("> ```diff")
                lines.extend("> " + line for line in comment.diff.splitlines())
                lines.extend(["> ```", ""])
            lines.extend([f"{grounding_tag(comment.grounding)} ({comment.severity}): {comment.problem.strip()}", "", f"**Fix.** {comment.fix.strip()}", ""])
    if document.retracted:
        lines.extend(["## Retracted by verification", ""])
        for comment_id, reason in document.retracted:
            lines.extend([f"- `{comment_id}`: {reason}", ""])
    return "\n".join(lines).rstrip() + "\n"


def write_assembled(document: ReviewDocument, path: Path, *, overwrite: bool = False) -> None:
    if path.exists() and not overwrite:
        raise AssemblyError(f"refusing to overwrite existing {path} (pass --overwrite)")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_markdown(document), encoding="utf-8")
