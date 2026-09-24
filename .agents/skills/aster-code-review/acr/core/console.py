"""Small terminal helpers for human-readable artifact locations."""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import TextIO


def _color(text: str, code: str, *, stream: TextIO) -> str:
    force = os.environ.get("ACR_FORCE_COLOR")
    if (os.environ.get("NO_COLOR") and not force) or not (stream.isatty() or force):
        return text
    return f"\033[{code}m{text}\033[0m"


def print_artifact(label: str, path: Path, *, stream: TextIO | None = None) -> None:
    """Print a labelled artifact path, using color when the terminal supports it."""
    stream = stream or sys.stdout
    prefix = _color(f"[acr] {label}:", "1;36", stream=stream)
    value = _color(str(path), "1;32", stream=stream)
    print(f"{prefix} {value}", file=stream, flush=True)
