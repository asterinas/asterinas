#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# aster_code_review.sh — run the aster-code-review skill headless, via an agent profile.
#
# The arguments ARE the skill's own argument string (see spec/interface.md),
# so this CLI is identical to triggering the skill inside an agent session:
#   aster_code_review.sh diff  <base>            <output> [--per-persona-context=…] [--overwrite]
#   aster_code_review.sh files <path[:lines] …>  <output> [--per-persona-context=…] [--overwrite]
#
# The agent is chosen by the environment, not an argument,
# because a given environment exposes only a few agents:
#   ACR_AGENT_PROFILE    REQUIRED. the agent profile to run under (see agent_profiles/).
#   ACR_PROFILE_VARIANT  optional. `smoke` for the low-effort overlay.
#
# The skill reviews the CURRENT working tree (HEAD),
# so cd into the repo (or a scratch worktree) first.
# Used identically by the benchmark, the CI workflow, and humans wanting a one-shot headless run.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

[[ $# -ge 1 ]] || {
    echo "usage: ACR_AGENT_PROFILE=<name> aster_code_review.sh <diff|files> <args…> <output> [flags]" >&2
    exit 2; }

# Rebuild the skill's argument string from argv, PRESERVING token boundaries.
# resolve_target.sh re-tokenizes this string
# by splitting on whitespace except inside double quotes (no escapes),
# so a bare "$*" would collapse a quoted path that contains spaces
# — `files "a b.rs" out.md` would arrive as three tokens (`a`, `b.rs`, `out.md`), not two.
# Re-quote any token that contains whitespace
# so the tokenizer sees the same boundaries the shell gave us;
# tokens with a colon line-range stay bare so the range still parses.
# A literal double quote can't be represented in that quote-toggling grammar,
# so refuse it rather than corrupt it.
args=""
for tok in "$@"; do
    case "$tok" in
        *'"'*)          echo "aster_code_review.sh: a double quote in an argument is not supported: $tok" >&2; exit 2 ;;
        *[[:space:]]*)  tok="\"$tok\"" ;;
    esac
    args="${args:+$args }$tok"
done
prompt="Use the aster-code-review skill with these arguments: $args. Review this working tree."
exec "$HERE/scripts/run_agent.sh" "$prompt"
