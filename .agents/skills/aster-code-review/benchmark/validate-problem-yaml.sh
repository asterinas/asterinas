#!/usr/bin/env bash
#
# validate-problem-yaml.sh — schema-check benchmark/problems.yaml (model-free; no git, no model).
# Exits non-zero on any schema error.
# Run by run.sh (fail-closed) and by the tests.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
python3 - "$HERE/problems.yaml" <<'PY'
import re, sys
try:
    import yaml
except ImportError:
    print("validate-problem-yaml.sh: PyYAML not available"); sys.exit(2)

path = sys.argv[1]
PERSONAS = {"maintainability", "development", "security", "hardware", "documentation"}
KINDS = {"file", "commit_message", "whole_change"}
SEVERITIES = {"critical", "major", "minor", "nit"}
SHA_RE = re.compile(r"^[0-9a-f]{40}$")     # full object name — abbreviations are not git-fetchable
REMOTE_RE = re.compile(r"^(https?://|git@)")
errs = []

try:
    docs = yaml.safe_load(open(path))
except Exception as e:
    print(f"FATAL: cannot parse {path}: {e}"); sys.exit(2)
if not isinstance(docs, list):
    print("FATAL: top level must be a sequence of problems"); sys.exit(2)

ids = set()
for i, p in enumerate(docs):
    tag = (p.get("problem_id") if isinstance(p, dict) else None) or f"#{i}"
    def err(m, tag=tag): errs.append(f"[{tag}] {m}")
    if not isinstance(p, dict):
        err("not a mapping"); continue
    pid = p.get("problem_id")
    if not isinstance(pid, str) or not pid:
        err("problem_id missing or not a string")
    elif pid in ids:
        err("duplicate problem_id")
    else:
        ids.add(pid)
    if not isinstance(p.get("source"), str) or not p["source"].strip():
        err("source missing or not a non-empty string")
    rm = p.get("review_mode")
    if not isinstance(rm, dict):
        err("review_mode missing or not a map")
    else:
        present = [k for k in ("diff", "files") if k in rm]
        if present not in (["diff"], ["files"]):
            err(f"review_mode must have exactly one of diff/files, got {present or 'none'}")
        elif "diff" in rm:
            # diff mode: the change under review is a commit,
            # fetched by full SHA (from `remote`, default upstream).
            # No base_commit (base = commit^).
            v = rm["diff"]
            if not isinstance(v, dict):
                err("review_mode.diff must be a {commit, remote?} map")
            else:
                c = v.get("commit")
                if not isinstance(c, str) or not SHA_RE.match(c):
                    err("review_mode.diff.commit must be a full 40-char hex SHA")
                r = v.get("remote")   # OPTIONAL; run.sh defaults it to upstream
                if r is not None and (not isinstance(r, str) or not REMOTE_RE.match(r)):
                    err("review_mode.diff.remote, if present, must be a fetch URL (https:// or git@)")
                extra = set(v) - {"commit", "remote"}
                if extra:
                    err(f"review_mode.diff has unexpected keys: {sorted(extra)}")
            if "base_commit" in p:
                err("base_commit must be omitted in diff mode (base is commit^)")
        else:
            # files mode: named paths reviewed at base_commit (a snapshot).
            if not isinstance(p.get("base_commit"), str) or not p["base_commit"].strip():
                err("base_commit missing or not a non-empty string (required in files mode)")
            v = rm["files"]
            if not isinstance(v, list) or not v or not all(isinstance(x, str) and x for x in v):
                err("review_mode.files must be a non-empty list of path strings")
    ds = p.get("defects")
    if not isinstance(ds, list) or not ds:
        err("defects missing or empty")
    else:
        for j, d in enumerate(ds):
            if not isinstance(d, dict):
                err(f"defect[{j}] not a mapping"); continue
            t = d.get("target")
            if not isinstance(t, dict):
                err(f"defect[{j}] target missing")
            else:
                k = t.get("kind")
                if k not in KINDS:
                    err(f"defect[{j}] target.kind invalid: {k!r}")
                if k == "file" and not t.get("path"):
                    err(f"defect[{j}] target.path required when kind=file")
                if k != "file" and t.get("path"):
                    err(f"defect[{j}] target.path only allowed when kind=file")
            if d.get("persona") not in PERSONAS:
                err(f"defect[{j}] persona invalid: {d.get('persona')!r}")
            if not d.get("grounding"):
                err(f"defect[{j}] grounding missing")
            if d.get("severity") not in SEVERITIES:
                err(f"defect[{j}] severity invalid: {d.get('severity')!r} (want one of {sorted(SEVERITIES)})")
            if not d.get("desc"):
                err(f"defect[{j}] desc text missing")
            if not d.get("expectation"):
                err(f"defect[{j}] expectation text missing")
            neg = bool(d.get("is_negative", False))
            if neg and d.get("fix"):
                err(f"defect[{j}] fix must be omitted when is_negative: true")
            if not neg and not d.get("fix"):
                err(f"defect[{j}] fix required")

if errs:
    print("SCHEMA ERRORS:")
    for e in errs:
        print("  -", e)
    sys.exit(1)

n_diff = sum(1 for p in docs if "diff" in p.get("review_mode", {}))
n_files = len(docs) - n_diff
n_def = sum(len(p.get("defects", [])) for p in docs)
n_neg = sum(1 for p in docs for d in p.get("defects", []) if d.get("is_negative"))
print(f"OK: {len(docs)} problems ({n_diff} diff, {n_files} files), "
      f"{n_def} defects ({n_neg} negative), problem_ids unique")
PY
