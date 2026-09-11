"""Progressive guideline disclosure through the copied query tool."""

from __future__ import annotations

import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


class DisclosureError(RuntimeError):
    pass


@dataclass(frozen=True)
class Catalog:
    persona: str
    text: str
    digest: str


def _query(repo_root: Path, *args: str) -> str:
    script = Path(__file__).resolve().parents[1] / "scripts" / "print_guideline.py"
    env = os.environ.copy()
    env.setdefault("ACR_GUIDELINE_ROOT", str(repo_root))
    result = subprocess.run([sys.executable, str(script), *args], cwd=repo_root, env=env, capture_output=True, text=True, check=False)
    if result.returncode:
        raise DisclosureError(result.stderr.strip() or "guideline query failed")
    return result.stdout


def catalog(repo_root: Path, persona: str) -> Catalog:
    text = _query(repo_root, "catalog", persona).rstrip()
    marker = "digest="
    digest = text.split(marker, 1)[1].split()[0] if marker in text else ""
    if not digest:
        raise DisclosureError(f"catalog for {persona} has no digest")
    return Catalog(persona, text, digest)


def query_exact(repo_root: Path, persona: str, digest: str, short_names: list[str]) -> str:
    return _query(repo_root, "show", "--expect-digest", digest, persona, *short_names)
