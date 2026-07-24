#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# build_pass_prompt.sh — assemble a persona pass prompt DETERMINISTICALLY,
# ordered for prompt-cache reuse.
# The STABLE blocks come first and are byte-identical across passes and across reviews:
# the shared reviewer contract,
# then one block per persona (its template + complete guideline gist catalog).
# The VOLATILE review input comes LAST,
# so everything above it is a reusable cache prefix.
#
# The orchestrator must spawn the pass with this script's EXACT output
# — retyping or paraphrasing it loses the byte-identical prefix, and with it the cache hit.
#
#   one persona  => a fan-out pass   (--per-persona-context=yes/auto)
#   several      => the combined pass (--per-persona-context=no)
#
# Usage: build_pass_prompt.sh <input-file> <persona> [<persona> ...]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SKILLDIR="$(cd "$HERE/.." && pwd)"
QUERY="$SKILLDIR/scripts/guideline_query.py"
GROOT="$(python3 "$QUERY" root)"
DISCLOSURE="${ACR_GUIDELINE_DISCLOSURE:-progressive}"
case "$DISCLOSURE" in
    progressive|full) ;;
    *) echo "build_pass_prompt.sh: ACR_GUIDELINE_DISCLOSURE must be progressive or full" >&2; exit 2 ;;
esac

input="${1:-}"; shift || true
[[ -n "$input" && -f "$input" ]] || { echo "build_pass_prompt.sh: a readable <input-file> is required" >&2; exit 2; }
[[ $# -ge 1 ]] || { echo "build_pass_prompt.sh: at least one <persona> is required" >&2; exit 2; }

linked_guideline_pages() { # <guideline-page-path-relative-to-GROOT>
    python3 - "$1" "$GROOT/$1" <<'PY'
import posixpath
import re
import sys

root_page = sys.argv[1]
root_file = sys.argv[2]
base_dir = posixpath.dirname(root_page)
seen = set()

with open(root_file, encoding="utf-8") as file:
    page_text = file.read()

for link in re.findall(r"\[[^\]]+\]\(([^)]+)\)", page_text):
    link = link.split("#", 1)[0]
    if not link or "://" in link or link.startswith("#"):
        continue
    if link.endswith("/"):
        link += "README.md"
    path = posixpath.normpath(posixpath.join(base_dir, link))
    if not path.startswith("book/src/to-contribute/coding-guidelines/"):
        continue
    if path == root_page or path in seen:
        continue
    seen.add(path)
    print(path)
PY
}

inline_full_guidelines() { # <guideline-page-path-relative-to-GROOT>
    local gp="$1" gfile="$GROOT/$gp" sub
    [[ -f "$gfile" ]] || return 0

    printf 'Rely on the inlined guideline material below as the authoritative guideline context for this pass. Do not re-read `book/src/to-contribute/coding-guidelines/` from the worktree under review for guideline content.\n\n'
    printf -- '--- guideline page (%s) ---\n\n' "$gp"
    cat "$gfile"; printf '\n'

    while IFS= read -r sub; do
        [[ -f "$GROOT/$sub" ]] || continue
        printf '\n--- guideline subpage (%s) ---\n\n' "$sub"
        cat "$GROOT/$sub"; printf '\n'
    done < <(linked_guideline_pages "$gp")
}

# --- STABLE: the shared reviewer contract ------------------------------------
cat "$SKILLDIR/scripts/pass_contract.md"
printf '\n'

# --- STABLE: one block per persona (template + catalog, or full rollback) ------
for persona in "$@"; do
    pf="$SKILLDIR/personas/$persona.md"
    [[ -f "$pf" ]] || { echo "build_pass_prompt.sh: no such persona: $persona" >&2; exit 2; }
    printf '===== PERSONA: %s =====\n\n' "$persona"
    cat "$pf"; printf '\n'
    if [[ "$DISCLOSURE" == progressive ]]; then
        printf 'Rely on this complete gist catalog as the authoritative guideline index for this pass. Fetch exact rule text with the query command in the pass contract; do not read guideline content from the reviewed worktree directly.\n\n'
        python3 "$QUERY" catalog "$persona"
        printf '\n'
    else
        gp="$(grep -oE 'book/[A-Za-z0-9/_-]+README\.md' "$pf" | head -1 || true)"
        [[ -n "$gp" ]] && inline_full_guidelines "$gp"
    fi
done

# --- VOLATILE: the review input (LAST; everything above is the cache prefix) ---
printf '===== REVIEW INPUT =====\n\n'
cat "$input"
