"""Versioned structured-output contracts used at stage boundaries."""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator


PERSONAS = ("maintainability", "development", "security", "hardware", "documentation")
SEVERITIES = ("critical", "major", "minor", "nit")
Verdict = Literal["confirmed", "uncertain", "refuted"]


class ContractModel(BaseModel):
    model_config = ConfigDict(extra="forbid", str_strip_whitespace=True)


class ReviewComment(ContractModel):
    file: str = Field(
        min_length=1,
        description="Repository-relative path, or a commit locus for a commit-message finding.",
    )
    line: int | None = Field(
        default=None,
        ge=1,
        description="Post-change or source line for a code finding; leave unset only for commit-message findings.",
    )
    persona: Literal["maintainability", "development", "security", "hardware", "documentation"] = Field(
        description="Persona that owns this finding."
    )
    grounding: str = Field(
        min_length=1,
        description="Fetched guideline short-name, or a plain-language defect class when no guideline applies.",
    )
    severity: Literal["critical", "major", "minor", "nit"] = Field(
        description="Fix priority: must fix, should fix, worth fixing, or optional/stylistic."
    )
    problem: str = Field(min_length=1, description="Concrete defect and its consequence in Markdown.")
    fix: str = Field(min_length=1, description="Concrete proposed remedy in Markdown.")
    diff: str | None = Field(
        default=None,
        description="Minimal relevant diff hunk or source excerpt when useful.",
    )

    @field_validator("file")
    @classmethod
    def relative_file(cls, value: str) -> str:
        if value.startswith("/"):
            raise ValueError("file must be repository-relative")
        return value


class CommentsEnvelope(ContractModel):
    comments: list[ReviewComment] = Field(description="All in-scope findings; empty only after a complete audit.")


class VerificationItem(ContractModel):
    comment_id: str = Field(min_length=1)
    verdict: Verdict
    premise: str = Field(min_length=1)
    evidence: list[str] = Field(default_factory=list)
    reason: str = Field(min_length=1)


class VerificationEnvelope(ContractModel):
    items: list[VerificationItem]


class ConsolidationResult(ContractModel):
    shared_fixes: dict[str, str] = Field(default_factory=dict)
    comment_updates: dict[str, str] = Field(default_factory=dict)

    @field_validator("shared_fixes", "comment_updates")
    @classmethod
    def nonempty_values(cls, value: dict[str, str]) -> dict[str, str]:
        if any(not key or not isinstance(text, str) or not text.strip() for key, text in value.items()):
            raise ValueError("fix mappings must have non-empty keys and values")
        return value


class SummaryResult(ContractModel):
    summary: str = Field(min_length=1)


class GraderItem(ContractModel):
    defect: int = Field(ge=1)
    status: Literal["caught", "partial", "miss"]
    reason: str = Field(min_length=1)


class GraderEnvelope(ContractModel):
    results: list[GraderItem]


def validate_fragment(value: object) -> list[ReviewComment]:
    """Validate a persona response; never turn malformed output into ``[]``."""

    if not isinstance(value, list):
        raise ValueError("persona fragment must be a JSON array")
    return [ReviewComment.model_validate(item) for item in value]
