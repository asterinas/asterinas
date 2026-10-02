"""Final Markdown sink with explicit overwrite protection."""

from __future__ import annotations

from pathlib import Path

from .assemble import ReviewDocument, render_markdown


class PublishError(RuntimeError):
    code = "PUBLISH_ERROR"


def publish(document: ReviewDocument, output: Path, *, repo_root: Path, overwrite: bool = False) -> Path:
    path = output if output.is_absolute() else repo_root / output
    if path.exists() and not overwrite:
        raise PublishError(f"refusing to overwrite existing {path} (pass --overwrite)")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_markdown(document, summary=document.summary or "No review findings were produced."), encoding="utf-8")
    return path
