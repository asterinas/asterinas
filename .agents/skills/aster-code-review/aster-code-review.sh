#!/usr/bin/env bash
#
# aster-code-review.sh — run the aster-code-review skill headless, via an agent profile.
#
# The arguments ARE the skill's own argument string (see spec/interface.md),
# so this CLI is identical to triggering the skill inside an agent session:
#   aster-code-review.sh diff  <base>            <output> [--per-persona-context=…] [--overwrite]
#   aster-code-review.sh files <path[:lines] …>  <output> [--per-persona-context=…] [--overwrite]
#
# The agent is chosen by the environment, not an argument, because a given
# environment exposes only a few agents:
#   ACR_AGENT_PROFILE    REQUIRED. the agent profile to run under (see agent_profiles/).
#   ACR_PROFILE_VARIANT  optional. `smoke` for the low-effort overlay.
#
# The skill reviews the CURRENT working tree (HEAD), so cd into the repo (or a
# scratch worktree) first. Used identically by the benchmark, the CI workflow,
# and humans wanting a one-shot headless run.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

[[ $# -ge 1 ]] || {
    echo "usage: ACR_AGENT_PROFILE=<name> aster-code-review.sh <diff|files> <args…> <output> [flags]" >&2
    exit 2; }

# Everything is passed through verbatim as the skill's argument string.
prompt="Use the aster-code-review skill with these arguments: $*. Review this working tree."
exec "$HERE/scripts/run-agent.sh" "$prompt"
