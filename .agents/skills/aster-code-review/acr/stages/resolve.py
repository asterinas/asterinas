"""Resolve the raw interface using the copied legacy deterministic script."""

from __future__ import annotations

import asyncio
import hashlib
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

from ..core.state import RunContext


class ResolveError(RuntimeError):
    code = "TARGET_ERROR"


@dataclass(frozen=True)
class ResolvedTarget:
    raw_args: str
    meta: dict[str, str]
    canonical_input: str
    reviewed_paths: tuple[str, ...]
    input_sha256: str
    repo_root: Path

    @property
    def mode(self) -> str:
        return self.meta["mode"]

    @property
    def output(self) -> Path:
        return Path(self.meta["output"])


def _repo_root(start: Path | None = None) -> Path:
    completed = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=start, capture_output=True, text=True, check=False)
    if completed.returncode:
        raise ResolveError(completed.stderr.strip() or "not inside a git repository")
    return Path(completed.stdout.strip()).resolve()


def _script_root() -> Path:
    return Path(__file__).resolve().parents[1] / "scripts"


def _run_script(root: Path, name: str, *args: str) -> str:
    env = os.environ.copy()
    env.setdefault("ACR_GUIDELINE_ROOT", str(root))
    script = _script_root() / name
    command = ["bash", str(script), *args] if script.suffix == ".sh" else [str(script), *args]
    result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True, check=False)
    if result.returncode:
        raise ResolveError(result.stderr.strip() or f"{name} failed with status {result.returncode}")
    return result.stdout


def _parse_meta(output: str) -> dict[str, str]:
    meta: dict[str, str] = {}
    for line in output.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        meta[key] = value
    if meta.get("mode") not in {"diff", "files"} or "output" not in meta:
        raise ResolveError("resolve_target did not return valid metadata")
    return meta


def _file_paths(meta: dict[str, str]) -> tuple[str, ...]:
    value = meta.get("files", "")
    paths: list[str] = []
    for item in value.split(",") if value else []:
        path = item.rsplit(":", 1)[0] if ":" in item else item
        if path and path not in paths:
            paths.append(path)
    return tuple(paths)


def _diff_paths(root: Path, meta: dict[str, str]) -> tuple[str, ...]:
    base = meta.get("base")
    if not base:
        raise ResolveError("diff metadata has no base")
    result = subprocess.run(["git", "diff", "--name-only", f"{base}..HEAD"], cwd=root, capture_output=True, text=True, check=False)
    if result.returncode:
        raise ResolveError(result.stderr.strip() or "cannot determine changed paths")
    return tuple(path for path in result.stdout.splitlines() if path)


def resolve_target(raw_args: str, *, context: RunContext | None = None, repo_root: Path | None = None) -> ResolvedTarget:
    root = _repo_root(repo_root)
    meta = _parse_meta(_run_script(root, "resolve_target.sh", "--meta", raw_args))
    canonical = _run_script(root, "resolve_target.sh", raw_args)
    paths = _diff_paths(root, meta) if meta["mode"] == "diff" else _file_paths(meta)
    target = ResolvedTarget(raw_args, meta, canonical, paths, hashlib.sha256(canonical.encode("utf-8")).hexdigest(), root)
    if context is not None:
        context.write_text("artifacts/canonical-input.txt", canonical)
        context.manifest.update({
            "mode": target.mode,
            "base": meta.get("base"),
            "files": meta.get("files"),
            "head": meta.get("head"),
            "branch": meta.get("branch"),
            "output": meta.get("output"),
            "overwrite": meta.get("overwrite") == "1",
            "canonical_input_sha256": target.input_sha256,
        })
        context.save()
    return target
