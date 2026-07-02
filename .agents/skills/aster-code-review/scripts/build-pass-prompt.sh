#!/usr/bin/env bash
#
# build-pass-prompt.sh — assemble a persona pass prompt DETERMINISTICALLY,
# ordered for prompt-cache reuse.
# The STABLE blocks come first and are byte-identical across passes and across reviews:
# the shared reviewer contract,
# then one block per persona (its template + its inlined guideline page).
# The VOLATILE review input comes LAST, so everything above it is a reusable cache prefix.
#
# The orchestrator must spawn the pass with this script's EXACT output
# — retyping or paraphrasing it loses the byte-identical prefix, and with it the cache hit.
#
#   one persona  => a fan-out pass   (--per-persona-context=yes/auto)
#   several      => the combined pass (--per-persona-context=no)
#
# Usage: build-pass-prompt.sh <input-file> <persona> [<persona> ...]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SKILLDIR="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$SKILLDIR/../../.." && pwd)"   # .agents/skills/aster-code-review -> repo root
# Where to read the guideline pages from.
# Defaults to the repo; the benchmark overrides it (ACR_GUIDELINE_ROOT) to a guidelines-only tree,
# so a historical worktree is reviewed with current guidelines without polluting its diff.
GROOT="${ACR_GUIDELINE_ROOT:-$REPO}"

input="${1:-}"; shift || true
[[ -n "$input" && -f "$input" ]] || { echo "build-pass-prompt.sh: a readable <input-file> is required" >&2; exit 2; }
[[ $# -ge 1 ]] || { echo "build-pass-prompt.sh: at least one <persona> is required" >&2; exit 2; }

# --- STABLE: the shared reviewer contract ------------------------------------
cat "$SKILLDIR/scripts/pass-contract.md"
printf '\n'

# --- STABLE: one block per persona (template + inlined guideline page) --------
for persona in "$@"; do
    pf="$SKILLDIR/personas/$persona.md"
    [[ -f "$pf" ]] || { echo "build-pass-prompt.sh: no such persona: $persona" >&2; exit 2; }
    printf '===== PERSONA: %s =====\n\n' "$persona"
    cat "$pf"; printf '\n'
    gp="$(grep -oE 'book/[A-Za-z0-9/_-]+README\.md' "$pf" | head -1 || true)"
    if [[ -n "$gp" && -f "$GROOT/$gp" ]]; then
        printf -- '--- guideline page (%s) ---\n\n' "$gp"
        cat "$GROOT/$gp"; printf '\n'
    fi
done

# --- VOLATILE: the review input (LAST; everything above is the cache prefix) ---
printf '===== REVIEW INPUT =====\n\n'
cat "$input"
