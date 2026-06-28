#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# parse_pr_command.sh — parse a `/aster-code-review …` PR-comment command into a validated CI plan.
# Reads the command text as $1, or the whole comment body on stdin.
# On success prints `key=value` lines to stdout;
# on an invalid command it prints an error to stderr and exits 2.
#
# Grammar (mirrors the skill interface, spec/interface.md):
#   /aster-code-review                      -> review the PR diff        (alias for `diff`)
#   /aster-code-review diff                 -> review the PR diff
#   /aster-code-review files <p1> … <pN>    -> review those files
#   /aster-code-review smoke     [--problems="…"]   -> the smoke test
#   /aster-code-review benchmark [--problems="…"]   -> the benchmark (informational in CI)
#
# The command mimics a CLI:
# `--problems="<id1> … <idN>"` selects a subset of benchmark problems (four-digit ids).
# The workflow translates it into the make knob `PROBLEMS=` internally;
# the user-facing flag is `--problems`.
#
# STRICT ALLOWLIST.
# This is a UX/routing layer with nice errors;
# it is NOT the security boundary.
# The workflow RE-VALIDATES every value that reaches a command
# (see .github/workflows/invoke_aster_code_review.yml),
# so even a PR-modified copy of this script cannot widen what the trusted workflow will run.
#
# Output keys:
#   kind=review  mode=diff
#   kind=review  mode=files   paths=<space-separated, validated>
#   kind=test    target=smoke|benchmark   problems=<empty | "NNNN …">
set -uo pipefail

fail() { printf 'parse_pr_command.sh: %s\n' "$1" >&2; exit 2; }

body="${1-}"
[ -n "$body" ] || body="$(cat)"

# The first line that starts (ignoring leading spaces) with the exact trigger token.
line="$(printf '%s\n' "$body" | grep -m1 -E '^[[:space:]]*/aster-code-review([[:space:]]|$)' || true)"
[ -n "$line" ] || fail "no '/aster-code-review' command found"

rest="$(printf '%s' "$line" | sed -E 's#^[[:space:]]*/aster-code-review[[:space:]]*##; s#[[:space:]]*$##')"
sub="${rest%%[[:space:]]*}"
args="$(printf '%s' "${rest#"$sub"}" | sed -E 's#^[[:space:]]*##')"
[ -n "$sub" ] || sub="diff"

case "$sub" in
    diff)
        [ -z "$args" ] || fail "'diff' takes no arguments"
        printf 'kind=review\nmode=diff\n' ;;
    files)
        [ -n "$args" ] || fail "'files' needs at least one path"
        for p in $args; do
            case "$p" in
                -*|/*|*..*) fail "invalid path: $p" ;;
            esac
            printf '%s' "$p" | grep -qE '^[A-Za-z0-9._/-]+$' || fail "invalid path: $p"
        done
        printf 'kind=review\nmode=files\npaths=%s\n' "$args" ;;
    smoke|benchmark)
        # Only --problems="…" is accepted, and only as space-separated problem numbers.
        # Validated in the main shell (not a subshell) so `fail` aborts the script.
        problems=""
        if [ -n "$args" ]; then
            case "$args" in
                --problems=*) problems="${args#--problems=}"; problems="${problems%\"}"; problems="${problems#\"}" ;;
                *) fail "'$sub' accepts only --problems=\"…\"" ;;
            esac
            printf '%s' "$problems" | grep -qE '^[0-9]{1,4}( [0-9]{1,4})*$' \
                || fail "--problems must be space-separated problem numbers, e.g. \"0002 0006\""
        fi
        printf 'kind=test\ntarget=%s\nproblems=%s\n' "$sub" "$problems" ;;
    *)
        fail "unknown subcommand '$sub' (use: diff | files | smoke | benchmark)" ;;
esac
