#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# overlay_skill.sh — copy TODAY's skill package into a scratch worktree,
# so a historical snapshot is reviewed by the *current* skill
# (the snapshot's own .agents are stale or absent).
# The overlaid files (.agents, .claude) are untracked at the historical base,
# so they do NOT show up in a `diff`-mode review.
#
# Guideline pages are deliberately NOT copied into the worktree's tracked book/:
# overwriting the snapshot's tracked book/ would pollute a `diff`-mode review.
# Instead, bundle a guidelines-only snapshot inside the overlaid skill package.
# build_pass_prompt.sh and guideline_query.py use that snapshot by default,
# so catalog construction and later rule queries still use
# current guidelines even if the review agent does not preserve ACR_GUIDELINE_ROOT
# when it later runs shell commands.
# guideline-root.required makes a missing snapshot fail closed instead of
# silently falling back to the historical worktree's book/ tree.
#
# CRITICAL (benchmark integrity):
# this EXCLUDES benchmark/, which holds problems.yaml — the answer key.
# The review agent must never see it.
set -euo pipefail
wt="${1:?usage: overlay_skill.sh <worktree>}"
HERE="$(cd "$(dirname "$0")" && pwd)"     # .../aster-code-review/benchmark
SKILL="$(cd "$HERE/.." && pwd)"           # .../aster-code-review

rm -rf "$wt/.agents" "$wt/.claude"
mkdir -p "$wt/.agents/skills"
cp -r "$SKILL" "$wt/.agents/skills/aster-code-review"
rm -rf "$wt/.agents/skills/aster-code-review/benchmark"   # <-- drop the answer key
rm -rf "$wt/.agents/skills/aster-code-review/guideline-root"
touch "$wt/.agents/skills/aster-code-review/guideline-root.required"
mkdir -p "$wt/.agents/skills/aster-code-review/guideline-root/book/src/to-contribute"
cp -r "$HERE/../../../../book/src/to-contribute/coding-guidelines" \
    "$wt/.agents/skills/aster-code-review/guideline-root/book/src/to-contribute/"

# Claude Code discovers skills under .claude/skills; mirror the repo's symlink layout.
mkdir -p "$wt/.claude/skills"
ln -sfn "../../.agents/skills/aster-code-review" "$wt/.claude/skills/aster-code-review"
