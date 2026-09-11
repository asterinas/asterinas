"""Deterministic path-based persona activation."""

from __future__ import annotations

from pathlib import Path
from typing import Protocol, Sequence

from .contracts import PERSONAS


class ActivationError(ValueError):
    pass


class ActivationPolicy(Protocol):
    def for_paths(self, paths: Sequence[str], *, repo_root: Path | None = None) -> tuple[str, ...]: ...


def _is_code(path: str) -> bool:
    return Path(path).suffix.lower() in {
        ".rs", ".c", ".h", ".cc", ".cpp", ".S", ".s", ".asm", ".nix", ".py", ".sh", ".toml",
        ".yaml", ".yml", ".json", ".xml", ".cfg", ".conf", ".lock",
    } or "/src/" in f"/{path}"


class DeterministicActivationPolicy:
    """Match the legacy activation table without model-assisted triage."""

    def for_paths(self, paths: Sequence[str], *, repo_root: Path | None = None) -> tuple[str, ...]:
        normalized = [str(path).replace("\\", "/") for path in paths]
        if not normalized:
            raise ActivationError("at least one reviewed path is required")
        code = any(_is_code(path) for path in normalized)
        active: list[str] = []
        if code:
            active.extend(("maintainability", "development", "security"))

        hardware = any(
            Path(path).suffix.lower() in {".s", ".S", ".asm"}
            or "/arch/" in f"/{path}"
            or path.startswith("arch/")
            for path in normalized
        )
        if repo_root is not None and not hardware:
            for path in normalized:
                candidate = repo_root / path
                try:
                    text = candidate.read_text(encoding="utf-8")
                except (OSError, UnicodeDecodeError):
                    continue
                if "asm!" in text or "global_asm!" in text:
                    hardware = True
                    break
        if hardware:
            active.append("hardware")

        documentation = any(
            path.startswith("book/")
            or Path(path).suffix.lower() in {".md", ".scml"}
            or "/syscall" in f"/{path}"
            or path.startswith("kernel/src/syscall")
            for path in normalized
        )
        if documentation:
            active.append("documentation")
        return tuple(persona for persona in PERSONAS if persona in active)
