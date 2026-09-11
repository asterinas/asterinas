"""Deterministic path-based persona activation."""

from __future__ import annotations

from pathlib import Path
from typing import Protocol, Sequence

from .contracts import PERSONAS


class ActivationError(ValueError):
    pass


class ActivationPolicy(Protocol):
    def for_paths(self, paths: Sequence[str], *, repo_root: Path | None = None) -> tuple[str, ...]: ...


class DeterministicActivationPolicy:
    """Always activate the three general personas; add specialists by path."""

    def for_paths(self, paths: Sequence[str], *, repo_root: Path | None = None) -> tuple[str, ...]:
        normalized = [str(path).replace("\\", "/") for path in paths]
        if not normalized:
            raise ActivationError("at least one reviewed path is required")
        active = ["maintainability", "development", "security"]

        hardware = any(
            Path(path).suffix.lower() in {".s", ".S", ".asm"}
            or "/arch/" in f"/{path}"
            or path.startswith("arch/")
            for path in normalized
        )
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
