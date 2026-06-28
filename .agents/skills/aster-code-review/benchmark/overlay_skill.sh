#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# overlay_skill.sh — copy TODAY's skill package into a scratch worktree,
# so a historical snapshot is reviewed by the *current* skill
# (the snapshot's own .agents are stale or absent).
# The overlaid files (.agents, .claude) are untracked at the historical base,
# so they do NOT show up in a `diff`-mode review.
#
# Guideline pages are deliberately NOT copied here:
# overwriting the snapshot's tracked book/ would pollute a `diff`-mode review.
# The harness instead points the skill at current guidelines via ACR_GUIDELINE_ROOT
# (a guidelines-only tree).
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

# Claude Code discovers skills under .claude/skills; mirror the repo's symlink layout.
mkdir -p "$wt/.claude/skills"
ln -sfn "../../.agents/skills/aster-code-review" "$wt/.claude/skills/aster-code-review"
