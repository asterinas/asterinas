"""Project replayed Responses messages onto the SDK's protocol schema.

Unknown envelope metadata is ignored without validating or rewriting field
values. Opaque payloads, including tool arguments and result text, stay intact.
"""

from __future__ import annotations

from functools import cache
from typing import Any

from openai.types.responses import ResponseInputItemParam
from pydantic import TypeAdapter


def normalize_responses_input(items: list[Any]) -> list[Any]:
    schema = _input_schema()
    return _project(items, [schema], schema.get("$defs", {}))


@cache
def _input_schema() -> dict[str, Any]:
    # The installed SDK owns the field definitions, including future additions.
    return TypeAdapter(list[ResponseInputItemParam]).json_schema()


def _project(value: Any, schemas: list[dict], definitions: dict) -> Any:
    schemas = list(_alternatives(schemas, definitions))
    if isinstance(value, list):
        item_schemas = [schema["items"] for schema in schemas if "items" in schema]
        return [_project(item, item_schemas, definitions) for item in value] if item_schemas else value
    if not isinstance(value, dict):
        return value

    fields: dict[str, list[dict]] = {}
    for schema in schemas:
        properties = schema.get("properties", {})
        # Select protocol variants without imposing required-field validation.
        if any(
            (key in value and not _matches_tag(value[key], properties.get(key, {}), definitions))
            or (key not in value and key in schema.get("required", []))
            for key in ("type", "role")
        ):
            continue
        for key, field_schema in properties.items():
            fields.setdefault(key, []).append(field_schema)

    # Unstructured payloads and unknown item types belong to the SDK/provider.
    if not fields:
        return value
    return {key: _project(item, fields[key], definitions) for key, item in value.items() if key in fields}


def _matches_tag(value: Any, schema: dict, definitions: dict) -> bool:
    return any(
        ("const" not in option or value == option["const"])
        and ("enum" not in option or value in option["enum"])
        and (option.get("type") != "null" or value is None)
        for option in _alternatives([schema], definitions)
    )


def _alternatives(schemas: list[dict], definitions: dict):
    for schema in schemas:
        while "$ref" in schema:
            schema = definitions[schema["$ref"].rsplit("/", 1)[-1]]
        alternatives = schema.get("anyOf", schema.get("oneOf"))
        if alternatives is not None:
            yield from _alternatives(alternatives, definitions)
        else:
            yield schema
