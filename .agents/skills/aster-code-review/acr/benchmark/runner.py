"""Benchmark control plane with detached worktrees and optional SDK grading.

Ground truth is loaded only by the controller.  It is never included in the
reviewer's worktree or review input; ``--grade`` starts a separate grader run
after the reviewer has published its document.
"""

from __future__ import annotations

import asyncio
import argparse
import json
import os
import re
import secrets
import shutil
import tempfile
from dataclasses import replace
from pathlib import Path
from typing import Literal

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    ValidationError,
    ValidationInfo,
    field_validator,
    model_validator,
)

from ..config import RunConfig, load_config
from ..agents.backend_factory import close_backend, create_backend
from ..core.orchestrator import run_review
from ..core.console import print_artifact
from ..core.events import EventLogger
from ..core.state import gmt8_timestamp
from .grader import RecallReport, calculate_recall, grade
from .worktree import WorktreeManager


class BenchmarkError(RuntimeError):
    pass


UTC_OFFSET_SUFFIX = re.compile(r"[+-]\d{4}$")


def benchmark_run_prefix(timestamp: str, problems: list[BenchmarkProblem]) -> str:
    """Return the user-visible directory prefix for a benchmark invocation."""
    local_timestamp = UTC_OFFSET_SUFFIX.sub("", timestamp)
    numeric_ids = [problem.problem_id.partition("-")[0] for problem in problems]
    if len(problems) == 1:
        selection = numeric_ids[0]
    else:
        selection = f"multi-{'_'.join(numeric_ids)}"
    return f"run-{local_timestamp}-{selection}-"


def overlay_package(source_package: Path, worktree: Path) -> Path:
    """Replace historical agent resources with an answer-free reviewer package."""
    source = source_package.resolve()
    worktree_root = worktree.resolve()
    for name in (".agents", ".claude"):
        path = worktree_root / name
        if path.is_symlink() or path.is_file():
            path.unlink()
        elif path.exists():
            shutil.rmtree(path)

    destination = (
        worktree_root / ".agents" / "skills" / "aster-code-review" / "acr"
    )
    destination.parent.mkdir(parents=True, exist_ok=True)

    def ignore(directory: str, names: list[str]):
        if Path(directory).name == "acr" and "benchmark" in names:
            return ["benchmark"]
        return [name for name in names if name.endswith(".pyc") or name == "__pycache__"]

    shutil.copytree(source, destination, ignore=ignore)
    return destination


def load_problems(
    path: Path, *, default_remote: str | None
) -> list[BenchmarkProblem]:
    """Load and fully validate the benchmark problem corpus."""
    try:
        import yaml
    except ImportError as exc:
        raise BenchmarkError("PyYAML is required to load benchmark problems; install acr[benchmark]") from exc
    try:
        values = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, yaml.YAMLError) as exc:
        raise BenchmarkError(f"cannot load benchmark problems: {exc}") from exc
    if not isinstance(values, list):
        raise BenchmarkError("benchmark problems must be a YAML sequence")
    problems = [
        BenchmarkProblem.from_mapping(value, default_remote=default_remote)
        for value in values
    ]

    ids: set[str] = set()
    numeric_ids: dict[str, str] = {}
    for problem in problems:
        if problem.problem_id in ids:
            raise BenchmarkError(f"duplicate problem_id: {problem.problem_id}")
        ids.add(problem.problem_id)
        numeric_id = NUMERIC_ID_RE.match(problem.problem_id).group(1)
        previous = numeric_ids.get(numeric_id)
        if previous is not None:
            raise BenchmarkError(
                f"numeric id {numeric_id!r} collides with {previous!r}"
            )
        numeric_ids[numeric_id] = problem.problem_id
    return problems


class BenchmarkSchema(BaseModel):
    model_config = ConfigDict(extra="allow", frozen=True)


class BenchmarkTarget(BenchmarkSchema):
    kind: Literal["file", "commit_message", "whole_change"]
    path: str | None = None

    @model_validator(mode="after")
    def validate_path(self) -> "BenchmarkTarget":
        if self.kind == "file" and not self.path:
            raise ValueError("target.path is required when kind=file")
        if self.kind != "file" and self.path:
            raise ValueError("target.path is only allowed when kind=file")
        return self


class BenchmarkDefect(BenchmarkSchema):
    target: BenchmarkTarget
    persona: Literal[
        "maintainability", "development", "security", "hardware", "documentation"
    ]
    grounding: str = Field(min_length=1)
    severity: Literal["critical", "major", "minor", "nit"]
    desc: str = Field(min_length=1)
    expectation: str = Field(min_length=1)
    fix: str | None = None
    is_negative: bool = False

    @model_validator(mode="after")
    def validate_fix(self) -> "BenchmarkDefect":
        if self.is_negative and self.fix:
            raise ValueError("fix must be omitted when is_negative is true")
        if not self.is_negative and not self.fix:
            raise ValueError("fix is required when is_negative is false")
        return self


class DiffReviewMode(BenchmarkSchema):
    model_config = ConfigDict(extra="forbid")
    base: str = Field(min_length=1)

    @field_validator("base")
    @classmethod
    def nonempty_base(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("base must not be blank")
        return value


class BenchmarkReviewMode(BenchmarkSchema):
    diff: DiffReviewMode | None = None
    files: tuple[str, ...] | None = Field(default=None, min_length=1)

    @model_validator(mode="before")
    @classmethod
    def exactly_one_mode(cls, value: object) -> object:
        if isinstance(value, dict):
            present = [name for name in ("diff", "files") if name in value]
            if len(present) != 1:
                raise ValueError(
                    f"review_mode must have exactly one of diff/files, got {present or 'none'}"
                )
        return value

    @field_validator("files")
    @classmethod
    def nonempty_file_paths(
        cls, value: tuple[str, ...] | None
    ) -> tuple[str, ...] | None:
        if value is not None and any(not path for path in value):
            raise ValueError("files must contain non-empty path strings")
        return value


NUMERIC_ID_RE = re.compile(r"^(\d+)")
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
REMOTE_RE = re.compile(r"^(https?://|git@)")


class BenchmarkProblem(BenchmarkSchema):
    problem_id: str
    source: str
    commit: str
    review_mode: BenchmarkReviewMode
    remote: str | None = None
    defects: tuple[BenchmarkDefect, ...] = Field(min_length=1)

    @model_validator(mode="before")
    @classmethod
    def reject_obsolete_fields_and_apply_remote(
        cls, value: object, info: ValidationInfo
    ) -> object:
        if not isinstance(value, dict):
            return value
        if "base_commit" in value:
            raise ValueError(
                "base_commit is obsolete; use the top-level commit instead"
            )
        review_mode = value.get("review_mode")
        if (
            isinstance(review_mode, dict)
            and "diff" in review_mode
            and "remote" not in value
        ):
            default_remote = (info.context or {}).get("default_remote")
            if default_remote is None:
                raise ValueError(
                    "diff benchmark problem requires benchmark.remote in the ACR config"
                )
            value = {**value, "remote": default_remote}
        return value

    @field_validator("problem_id")
    @classmethod
    def valid_problem_id(cls, value: str) -> str:
        if not value:
            raise ValueError("problem_id must not be empty")
        if NUMERIC_ID_RE.match(value) is None:
            raise ValueError("problem_id must begin with a numeric part")
        return value

    @field_validator("source", "commit")
    @classmethod
    def nonempty_text(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("value must not be blank")
        return value

    @field_validator("remote")
    @classmethod
    def valid_remote(cls, value: str | None) -> str | None:
        if value is not None and REMOTE_RE.match(value) is None:
            raise ValueError("remote must be a fetch URL (https:// or git@)")
        return value

    @model_validator(mode="after")
    def validate_diff_commit(self) -> "BenchmarkProblem":
        if self.review_mode.diff is not None and FULL_SHA_RE.match(self.commit) is None:
            raise ValueError("commit must be a full 40-character hex SHA in diff mode")
        return self

    @classmethod
    def from_mapping(
        cls, value: dict, *, default_remote: str | None = None
    ) -> "BenchmarkProblem":
        try:
            return cls.model_validate(
                value, context={"default_remote": default_remote}
            )
        except ValidationError as exc:
            problem_id = value.get("problem_id", "<unknown>") if isinstance(value, dict) else "<unknown>"
            raise BenchmarkError(
                f"invalid benchmark problem {problem_id}: {exc}"
            ) from exc

    @property
    def mode(self) -> Literal["diff", "files"]:
        return "diff" if self.review_mode.diff is not None else "files"

    @property
    def base(self) -> str | None:
        return self.review_mode.diff.base if self.review_mode.diff is not None else None

    @property
    def files(self) -> tuple[str, ...]:
        return self.review_mode.files or ()

    def raw_args(self, output: Path) -> str:
        if self.mode == "diff":
            return f"diff {self.base} {output} --overwrite"
        return "files " + " ".join(self.files) + f" {output} --overwrite"

    def expected_text(self) -> str:
        lines = ["# Expected defects", ""]
        for index, defect in enumerate(self.expected_defects(), 1):
            location = defect.target.path or f"<{defect.target.kind}>"
            lines.extend((f"{index}. location: {location} (persona: {defect.persona}, grounding: {defect.grounding}, severity: {defect.severity})", f"   MATCH IF: {' '.join(defect.expectation.split())}", ""))
        return "\n".join(lines)

    def expected_defects(self) -> tuple[BenchmarkDefect, ...]:
        return tuple(item for item in self.defects if not item.is_negative)

    def defect_label(self, defect_id: int) -> str:
        defect = self.expected_defects()[defect_id - 1]
        location = defect.target.path or f"<{defect.target.kind}>"
        expectation = " ".join(defect.expectation.split())
        return f"{location}: {expectation}"


def print_recall(problem: BenchmarkProblem, report: RecallReport) -> None:
    percent = report.recall * 100
    print(
        f"[acr] Recall {problem.problem_id}: {percent:.2f}% "
        f"({report.caught}/{report.expected} caught; "
        f"partial={report.partial}, miss={report.miss})"
    )
    if not report.not_caught:
        print(f"[acr] Defects not caught {problem.problem_id}: none")
        return
    print(f"[acr] Defects not caught {problem.problem_id}:")
    for item in report.not_caught:
        print(
            f"[acr]   defect {item.defect} [{item.status}] "
            f"{problem.defect_label(item.defect)} | grader: {item.reason}"
        )


async def run_problem(
    problem: BenchmarkProblem,
    config: RunConfig,
    *,
    repo_root: Path,
    work_root: Path,
    output_root: Path | None = None,
    worktree_name: str | None = None,
    timestamp: str | None = None,
) -> Path:
    manager = WorktreeManager(repo_root, work_root)
    worktree = manager.add(worktree_name or "wt", problem.commit, remote=problem.remote)
    timestamp = timestamp or gmt8_timestamp()
    output_root = output_root or work_root
    output_root.mkdir(parents=True, exist_ok=True)
    output = output_root / f"{problem.problem_id}-{timestamp}.review.md"
    try:
        # The overlay is deliberately caller-owned. The reviewer sees only the
        # skill package and checkout, never benchmark problem metadata.
        overlay_package(Path(__file__).resolve().parents[1], worktree)
        # Keep guideline data outside the historical checkout.  This mirrors
        # the shell harness overlay and prevents the snapshot's own book/ from
        # changing the review corpus.
        skill_root = repo_root.resolve()
        previous_guideline_root = os.environ.get("ACR_GUIDELINE_ROOT")
        os.environ["ACR_GUIDELINE_ROOT"] = str(skill_root)
        try:
            return await run_review(
                problem.raw_args(output),
                config,
                repo_root=worktree,
                benchmark_id=problem.problem_id,
            )
        finally:
            if previous_guideline_root is None:
                os.environ.pop("ACR_GUIDELINE_ROOT", None)
            else:
                os.environ["ACR_GUIDELINE_ROOT"] = previous_guideline_root
    finally:
        manager.remove(worktree)


async def _run_selected(
    problems: list[BenchmarkProblem],
    config: RunConfig,
    repo_root: Path,
    work_root: Path,
    output_root: Path,
    *,
    timestamp: str,
) -> list[Path]:
    outputs: list[Path] = []
    for index, problem in enumerate(problems, 1):
        outputs.append(
            await run_problem(
                problem,
                config,
                repo_root=repo_root,
                work_root=work_root,
                output_root=output_root,
                worktree_name=f"wt{index}",
                timestamp=timestamp,
            )
        )
    return outputs


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run the ACR benchmark with a detached worktree")
    parser.add_argument("--problems", type=Path, default=Path(__file__).with_name("problems.yaml"))
    parser.add_argument("--problem", action="append", dest="selectors", default=[])
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--work", type=Path, default=None)
    parser.add_argument("--backend", choices=("openai-agents", "pi-agent", "fake"), default=None)
    parser.add_argument("--config", type=Path, default=None, help="ACR TOML configuration; credentials are read from its api_key_env")
    parser.add_argument("--grade", action="store_true", help="run the isolated structured grader after each review")
    parser.add_argument("--timeout", type=float, default=None, help="per-agent timeout in seconds")
    parser.add_argument("--retries", type=int, default=None, help="agent retries")
    parser.add_argument("--max-turns", type=int, default=None, help="maximum SDK turns")
    parser.add_argument("--per-persona-context", choices=("yes", "no", "auto"), default=None)
    args = parser.parse_args(argv)
    try:
        config = load_config(
            args.config,
            overrides={"backend": args.backend} if args.backend else None,
        )
        problems = load_problems(
            args.problems, default_remote=config.benchmark_remote
        )
        if args.selectors:
            problems = [problem for problem in problems if any(problem.problem_id == selector or problem.problem_id.startswith(selector) for selector in args.selectors)]
        if not problems:
            raise BenchmarkError("no benchmark problems selected")
        timestamp = gmt8_timestamp()
        if args.work:
            work_root = args.work.resolve()
            work_root.mkdir(parents=True, exist_ok=True)
        else:
            if config.backend == "pi-agent":
                benchmark_root = Path(
                    os.environ.get(
                        "ACR_PI_BENCHMARK_ROOT",
                        Path(tempfile.gettempdir())
                        / "aster-code-review"
                        / "benchmarks-pi",
                    )
                ).resolve()
            else:
                benchmark_root = Path(os.environ.get("ACR_BENCHMARK_ROOT", Path(tempfile.gettempdir()) / "aster-code-review" / "benchmarks")).resolve()
            benchmark_root.mkdir(parents=True, exist_ok=True)
            if config.backend == "pi-agent":
                selection = problems[0].problem_id if len(problems) == 1 else "multi-" + "_".join(problem.problem_id for problem in problems)
                work_root = benchmark_root / f"run-{timestamp}-{selection}-{secrets.token_hex(6)}"
                work_root.mkdir(mode=0o700, exist_ok=False)
            else:
                work_root = Path(
                    tempfile.mkdtemp(
                        prefix=benchmark_run_prefix(timestamp, problems),
                        dir=benchmark_root,
                    )
                )
        if args.backend:
            config = config.with_backend(args.backend)
        config = replace(config, log_root=work_root / "logs")
        if args.timeout is not None:
            config = replace(config, timeout_seconds=args.timeout)
        if args.retries is not None:
            config = replace(config, retries=args.retries)
        if args.max_turns is not None:
            config = replace(config, max_turns=args.max_turns)
        if args.per_persona_context is not None:
            config = replace(config, per_persona_context=args.per_persona_context)
        config.validate()
        with tempfile.TemporaryDirectory(prefix="acr-benchmark-worktrees-") as scratch:
            outputs = asyncio.run(
                _run_selected(
                    problems,
                    config,
                    args.repo.resolve(),
                    Path(scratch),
                    work_root,
                    timestamp=timestamp,
                )
            )
        if args.grade:
            grader_run_id = f"grader-{timestamp}"
            grader_root = config.log_root / grader_run_id
            grader_root.mkdir(parents=True, mode=0o700, exist_ok=False)
            grader = create_backend(
                config,
                repo_root=args.repo.resolve(),
                run_root=grader_root,
            )
            grader_logger = EventLogger(
                grader_root,
                grader_run_id,
                benchmark_id=problems[0].problem_id if len(problems) == 1 else None,
                max_content_bytes=config.trace_max_content_bytes,
            )
            async def grade_all() -> list[RecallReport]:
                reports: list[RecallReport] = []
                for problem, output in zip(problems, outputs):
                    expected = problem.expected_text()
                    result = await grade(expected, output.read_text(encoding="utf-8"), grader, config, logger=grader_logger)
                    report = calculate_recall(expected, result)
                    reports.append(report)
                    grader_logger.sdk_trace(
                        "benchmark.recall",
                        stage="grader",
                        problem=problem.problem_id,
                        **report.as_dict(),
                    )
                    print(json.dumps({"problem": problem.problem_id, **report.as_dict(), "results": result.model_dump()["results"]}, ensure_ascii=False))
                    print_recall(problem, report)
                return reports
            async def grade_and_close() -> list[RecallReport]:
                try:
                    return await grade_all()
                finally:
                    await close_backend(grader)

            reports = asyncio.run(grade_and_close())
            expected_total = sum(report.expected for report in reports)
            caught_total = sum(report.caught for report in reports)
            print(
                f"[acr] Overall recall: {caught_total / expected_total * 100:.2f}% "
                f"({caught_total}/{expected_total} caught)"
            )
            print_artifact("Grader log", grader_logger.main_path)
            print_artifact("Grader SDK trace", grader_logger.sdk_trace_path)
    except (BenchmarkError, OSError, RuntimeError) as exc:
        print(f"acr benchmark: {exc}", file=__import__("sys").stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
