#!/usr/bin/env python3
"""Validate and normalize benchmark grader output without an SDK call."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path


EXPECTED_ID_RE = re.compile(r"^(?P<id>[1-9][0-9]*)\.\s+location:")
STATUSES = {"caught", "partial", "miss"}


def normalize(expected: Path, output: Path) -> dict:
    ids = [int(match.group("id")) for line in expected.read_text(encoding="utf-8").splitlines() if (match := EXPECTED_ID_RE.match(line))]
    if ids != list(range(1, len(ids) + 1)) or not ids:
        raise ValueError("expected defects must be numbered contiguously")
    value = json.loads(output.read_text(encoding="utf-8"))
    if not isinstance(value, list):
        raise ValueError("grader output must be a JSON array")
    seen: dict[int, dict] = {}
    for item in value:
        if not isinstance(item, dict) or set(item) != {"defect", "status", "reason"}:
            raise ValueError("each result must contain exactly defect, status, reason")
        defect = item["defect"]
        if isinstance(defect, bool) or not isinstance(defect, int) or defect in seen or defect not in ids:
            raise ValueError(f"invalid or duplicate defect id: {defect!r}")
        if item["status"] not in STATUSES or not isinstance(item["reason"], str) or not item["reason"].strip():
            raise ValueError(f"invalid result for defect {defect}")
        seen[defect] = {"defect": defect, "status": item["status"], "reason": " ".join(item["reason"].split())}
    if set(seen) != set(ids):
        raise ValueError("grader results do not cover expected defects exactly")
    results = [seen[index] for index in ids]
    counts = Counter(item["status"] for item in results)
    return {"caught": counts["caught"], "partial": counts["partial"], "miss": counts["miss"], "results": results}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", default="grade")
    parser.add_argument("expected", type=Path)
    parser.add_argument("grader_output", type=Path)
    args = parser.parse_args()
    try:
        result = normalize(args.expected, args.grader_output)
    except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as exc:
        print(f"parse_grade.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
