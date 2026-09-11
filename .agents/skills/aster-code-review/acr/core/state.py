"""Run state, checkpoints, and artifact layout."""

from __future__ import annotations

import hashlib
import json
import os
import secrets
import tempfile
from dataclasses import asdict, dataclass, field
from datetime import datetime, timedelta, timezone
from enum import StrEnum
from pathlib import Path
from typing import Any


GMT8 = timezone(timedelta(hours=8), name="GMT+8")


def gmt8_timestamp(value: datetime | None = None) -> str:
    """Return a filesystem-safe timestamp in GMT+8."""
    return (value or datetime.now(GMT8)).astimezone(GMT8).strftime("%Y%m%dT%H%M%S%z")


class Stage(StrEnum):
    CREATED = "CREATED"
    RESOLVED = "RESOLVED"
    ACTIVATED = "ACTIVATED"
    PROMPTS_BUILT = "PROMPTS_BUILT"
    PASSES_RUNNING = "PASSES_RUNNING"
    PASSES_VALIDATED = "PASSES_VALIDATED"
    ASSEMBLED = "ASSEMBLED"
    VERIFIED = "VERIFIED"
    CONSOLIDATED = "CONSOLIDATED"
    SUMMARIZED = "SUMMARIZED"
    PUBLISHED = "PUBLISHED"
    FAILED = "FAILED"
    CANCELLED = "CANCELLED"


class ErrorCode(StrEnum):
    CONFIG_ERROR = "CONFIG_ERROR"
    TARGET_ERROR = "TARGET_ERROR"
    ACTIVATION_ERROR = "ACTIVATION_ERROR"
    AGENT_TIMEOUT = "AGENT_TIMEOUT"
    AGENT_OUTPUT_INVALID = "AGENT_OUTPUT_INVALID"
    TOOL_DENIED = "TOOL_DENIED"
    TOOL_TIMEOUT = "TOOL_TIMEOUT"
    ASSEMBLY_ERROR = "ASSEMBLY_ERROR"
    VERIFICATION_ERROR = "VERIFICATION_ERROR"
    PUBLISH_ERROR = "PUBLISH_ERROR"
    WORKTREE_ERROR = "WORKTREE_ERROR"


@dataclass
class StageRecord:
    stage: str
    status: str
    started_at: str
    finished_at: str | None = None
    artifact: str | None = None
    artifact_sha256: str | None = None
    error_code: str | None = None


@dataclass
class RunState:
    schema_version: int
    run_id: str
    raw_args_sha256: str
    stage: str = Stage.CREATED
    records: list[StageRecord] = field(default_factory=list)
    activated_personas: list[str] = field(default_factory=list)
    artifact_digests: dict[str, str] = field(default_factory=dict)
    error_code: str | None = None
    error_message: str | None = None

    def begin(self, stage: Stage) -> StageRecord:
        record = StageRecord(stage=stage.value, status="running", started_at=_now())
        self.records.append(record)
        self.stage = stage.value
        return record

    def complete(self, record: StageRecord, *, artifact: Path | None = None) -> None:
        record.status = "completed"
        record.finished_at = _now()
        if artifact is not None:
            record.artifact = str(artifact)
            record.artifact_sha256 = sha256_file(artifact)
            self.artifact_digests[str(artifact)] = record.artifact_sha256

    def fail(self, code: str, message: str) -> None:
        self.stage = Stage.FAILED.value
        self.error_code = code
        self.error_message = message
        self.records.append(StageRecord(stage=Stage.FAILED.value, status="failed", started_at=_now(), finished_at=_now(), error_code=code))


def _now() -> str:
    return datetime.now(GMT8).isoformat()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2, sort_keys=True)
            stream.write("\n")
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


class RunContext:
    """Own all files belonging to one run and make state transitions durable."""

    def __init__(self, *, root: Path, state: RunState, manifest: dict[str, Any]):
        self.root = root
        self.state = state
        self.manifest = manifest

    @classmethod
    def create(cls, raw_args: str, config: Any, *, run_id: str | None = None, benchmark_id: str | None = None) -> "RunContext":
        created = datetime.now(GMT8)
        rid = run_id or f"acr-{gmt8_timestamp(created)}-{secrets.token_hex(6)}"
        root = Path(config.log_root) / rid
        root.mkdir(parents=True, mode=0o700, exist_ok=False)
        for relative in ("artifacts/prompts", "artifacts/fragments", "agents/personas", "errors"):
            (root / relative).mkdir(parents=True, mode=0o700, exist_ok=True)
        state = RunState(schema_version=1, run_id=rid, raw_args_sha256=sha256_text(raw_args))
        manifest = {
            "schema_version": 1,
            "run_id": rid,
            "benchmark_id": benchmark_id,
            "raw_args_sha256": state.raw_args_sha256,
            "backend": config.backend,
            "model": config.model,
            "review_model": config.review_model or config.model,
            "reasoning_effort": config.reasoning_effort,
            "postprocess_reasoning_effort": config.postprocess_reasoning_effort,
            "wire_api": config.wire_api,
            "max_turns": config.max_turns,
            "timeout_seconds": config.timeout_seconds,
            "agent_retries": config.retries,
            "model_retries": config.model_retries,
            "provider_adapter": config.provider.adapter,
            "pi": {
                "agent_dir": str(config.pi.agent_dir),
                "node_command": config.pi.node_command,
                "keep_native_sessions": config.pi.keep_native_sessions,
                "trusted_extensions": list(config.pi.trusted_extensions),
                "guideline_script": {
                    "path": str(
                        Path(__file__).resolve().parents[1]
                        / "scripts"
                        / "print_guideline.py"
                    ),
                    "sha256": hashlib.sha256(
                        (
                            Path(__file__).resolve().parents[1]
                            / "scripts"
                            / "print_guideline.py"
                        ).read_bytes()
                    ).hexdigest(),
                },
            }
            if config.backend == "pi-agent"
            else None,
            "per_persona_context": config.per_persona_context,
            "tools_enabled": config.tools_enabled,
            "tools": config.tools.as_dict(),
            "tracing_enabled": config.tracing_enabled,
            "trace_include_sensitive_data": config.trace_include_sensitive_data,
            "trace_max_content_bytes": config.trace_max_content_bytes,
            "disclosure": os.environ.get("ACR_GUIDELINE_DISCLOSURE", "progressive"),
            "created_at": created.isoformat(),
        }
        context = cls(root=root, state=state, manifest=manifest)
        context.save()
        return context

    @classmethod
    def resume(cls, root: Path) -> "RunContext":
        """Load a previously checkpointed run without changing its inputs."""
        root = root.resolve()
        state = load_state(root)
        manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
        if manifest.get("run_id") != state.run_id:
            raise ValueError("manifest and state run IDs differ")
        return cls(root=root, state=state, manifest=manifest)

    @property
    def artifact_dir(self) -> Path:
        return self.root / "artifacts"

    def artifact(self, name: str) -> Path:
        path = self.artifact_dir / name
        path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        return path

    def save(self) -> None:
        _write_json(self.root / "state.json", asdict(self.state))
        _write_json(self.root / "manifest.json", self.manifest)

    def write_text(self, relative: str, text: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        path.chmod(0o600)
        return path

    def write_json(self, relative: str, value: Any) -> Path:
        path = self.root / relative
        _write_json(path, value)
        path.chmod(0o600)
        return path


def load_state(root: Path) -> RunState:
    value = json.loads((root / "state.json").read_text(encoding="utf-8"))
    value["records"] = [StageRecord(**record) for record in value.get("records", [])]
    return RunState(**value)
