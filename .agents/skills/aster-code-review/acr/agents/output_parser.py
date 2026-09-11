"""Strict conversion of SDK/fake responses into ACR contracts."""

from __future__ import annotations

import json
from typing import Any, TypeVar

from pydantic import BaseModel

from ..core.contracts import CommentsEnvelope, ReviewComment, validate_fragment

T = TypeVar("T", bound=BaseModel)


class OutputParseError(ValueError):
    pass


def parse_json(value: Any) -> Any:
    if isinstance(value, str):
        try:
            return json.loads(value)
        except json.JSONDecodeError as exc:
            raise OutputParseError(f"agent output is not valid JSON: {exc}") from exc
    if isinstance(value, BaseModel):
        return value
    return value


def parse_model(value: Any, model: type[T]) -> T:
    value = parse_json(value)
    try:
        if isinstance(value, model):
            return value
        return model.model_validate(value)
    except Exception as exc:
        raise OutputParseError(f"agent output does not match {model.__name__}: {exc}") from exc


def parse_comments(value: Any) -> list[ReviewComment]:
    value = parse_json(value)
    if isinstance(value, CommentsEnvelope):
        return list(value.comments)
    if isinstance(value, dict) and set(value) == {"comments"}:
        return list(parse_model(value, CommentsEnvelope).comments)
    try:
        return validate_fragment(value)
    except Exception as exc:
        raise OutputParseError(str(exc)) from exc
