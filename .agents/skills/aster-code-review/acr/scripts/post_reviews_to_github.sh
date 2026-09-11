#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

#
# post_reviews_to_github.sh — post an aster-code-review review file to a GitHub PR.
#
# The review file is GitHub-agnostic (the skill never knows about a PR),
# so the PR coordinates are arguments, not frontmatter:
#
#   post_reviews_to_github.sh --repo <owner/repo> --pr <N> --head-sha <sha> \
#                             [--finalize] [--event comment|approve|request_changes] \
#                             <review-file>
#
# What it does:
#   1. Parses the review file
#      — the `# Summary` (+ any consolidated-fix blockquotes) becomes the review body;
#      each `### `path` line N` becomes an inline comment.
#      An AI-provenance note (a `> [!NOTE]` callout) is prepended to the body,
#      since everything this script posts is aster-code-review output.
#   2. Splits comments into inline (on the RIGHT side of the PR diff) and off-diff.
#      Off-diff comments cannot be inlined (GitHub silently discards those), so they
#      are surfaced in the review body under "Findings not attachable to this PR's
#      diff" — and logged — rather than lost.
#   3. Creates a PENDING review (body only) and adds the comments
#      via GraphQL addPullRequestReviewThread (reliable line/side positioning).
#   4. With --finalize, submits the review as --event (default: comment).
#      Without it, the review is left PENDING for a human to submit on GitHub.
#
# Needs the `gh` CLI, authenticated with `pull-requests: write`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

REPO="" PR="" HEAD_SHA="" FINALIZE=0 EVENT="comment" REVIEW_FILE=""
usage() { echo "usage: $0 --repo <owner/repo> --pr <N> --head-sha <sha> [--finalize] [--event comment|approve|request_changes] <review-file>" >&2; }
while [ $# -gt 0 ]; do
    case "$1" in
        --repo)      REPO="$2"; shift 2 ;;
        --pr)        PR="$2"; shift 2 ;;
        --head-sha)  HEAD_SHA="$2"; shift 2 ;;
        --event)     EVENT="$2"; shift 2 ;;
        --finalize)  FINALIZE=1; shift ;;
        -h|--help)   usage; exit 0 ;;
        -*)          echo "unknown option: $1" >&2; usage; exit 2 ;;
        *)           [ -z "$REVIEW_FILE" ] || { echo "unexpected arg: $1" >&2; exit 2; }; REVIEW_FILE="$1"; shift ;;
    esac
done
missing=""
[ -n "$REPO" ]      || missing="$missing --repo"
[ -n "$PR" ]        || missing="$missing --pr"
[ -n "$HEAD_SHA" ]  || missing="$missing --head-sha"
[ -n "$REVIEW_FILE" ] || missing="$missing <review-file>"
[ -z "$missing" ] || { echo "missing required:$missing" >&2; usage; exit 2; }
[ -f "$REVIEW_FILE" ] || { echo "error: review file not found: $REVIEW_FILE" >&2; exit 2; }
case "$EVENT" in
    comment) API_EVENT=COMMENT ;; approve) API_EVENT=APPROVE ;; request_changes) API_EVENT=REQUEST_CHANGES ;;
    *) echo "error: --event must be comment|approve|request_changes" >&2; exit 2 ;;
esac

echo "PR #$PR  repo $REPO  head $HEAD_SHA  event $EVENT$([ "$FINALIZE" = 1 ] && echo ' (finalize)' || echo ' (pending)')"

# --- staleness: the review must describe the PR's current head ---
CURRENT_SHA=$(gh pr view "$PR" --repo "$REPO" --json headRefOid -q .headRefOid 2>/dev/null || echo "")
if [ -z "$CURRENT_SHA" ]; then
    echo "warning: could not fetch current PR head; proceeding."
elif [ "$CURRENT_SHA" != "$HEAD_SHA" ]; then
    echo "error: PR advanced since the review was generated (review $HEAD_SHA, current $CURRENT_SHA). Re-review." >&2
    exit 1
fi

# --- parse the review file: summary body + inline comments, then drop out-of-diff comments ---
# The PR diff goes to parse_review.py via a FILE, not an env var:
# a large diff passed inline as DIFF=...
# overflows the exec argument list ("Argument list too long").
DIFF_TMP="$(mktemp)"; trap 'rm -f "$DIFF_TMP"' EXIT
gh pr diff "$PR" --repo "$REPO" > "$DIFF_TMP" 2>/dev/null || true
PARSED=$(REVIEW_FILE="$REVIEW_FILE" DIFF_FILE="$DIFF_TMP" python3 "$HERE/parse_review.py")

SUMMARY=$(echo "$PARSED"  | python3 -c 'import sys,json;print(json.load(sys.stdin)["summary"])')
N_KEEP=$(echo "$PARSED"   | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["comments"]))')
N_DROP=$(echo "$PARSED"   | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["dropped"]))')
COMMENTS=$(echo "$PARSED" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["comments"]))')
# Off-diff findings can't be inline comments; parse_review.py renders them as a
# Markdown section appended to the body so they aren't lost (empty when none).
DROPPED_SECTION=$(echo "$PARSED" | python3 -c 'import sys,json;print(json.load(sys.stdin)["dropped_section"])')

echo "parsed $N_KEEP inline comment(s) on diff lines; dropped $N_DROP off-diff."
if [ "$N_DROP" -gt 0 ]; then
    echo "$PARSED" | python3 -c '
import sys, json
for c in json.load(sys.stdin)["dropped"]:
    where = "file-level" if c.get("subject_type")=="file" else ("line "+str(c.get("line")))
    p = c["path"]
    print(f"  dropped (not in PR diff): {p} ({where})")'
fi
if [ "$N_KEEP" -eq 0 ] && [ -z "$SUMMARY" ] && [ "$N_DROP" -eq 0 ]; then
    echo "nothing to post (no summary, no in-diff comments, no off-diff findings)."; exit 0
fi

# Clear any leftover PENDING review from a previous failed run.
# GitHub allows only one pending review per user per PR,
# so a run that created the review then died before finalizing (submit/dismiss)
# would otherwise block every retry with a 422
# "User can only have one pending review per pull request".
# The API only returns the authenticated user's own pending review,
# so filtering on state is safe.
PENDING_IDS=$(gh api "repos/$REPO/pulls/$PR/reviews" --paginate --jq '.[] | select(.state=="PENDING") | .id' 2>/dev/null || true)
for rid in $PENDING_IDS; do
    echo "clearing leftover pending review $rid from an earlier run."
    gh api --method DELETE "repos/$REPO/pulls/$PR/reviews/$rid" >/dev/null 2>&1 || true
done

# --- create a PENDING review (AI note + summary + off-diff findings as body), then add comments ------
# The body is an AI-provenance note, then the summary (if any),
# then any off-diff findings that could not be inlined (the dropped section).
# Built in Python so the backticks/em-dash/arrows need no shell escaping;
# json.dumps emits \uXXXX for the non-ASCII glyphs,
# so it is safe under any locale in CI.
# The skill link is derived from $REPO (correct on forks) and points at HEAD
# — the repo's default-branch copy of the skill, i.e. the canonical one to consult.
PAYLOAD=$(SUMMARY="$SUMMARY" DROPPED_SECTION="$DROPPED_SECTION" HEAD_SHA="$HEAD_SHA" REPO="$REPO" python3 -c '
import json, os
repo = os.environ["REPO"]
url = f"https://github.com/{repo}/tree/HEAD/.agents/skills/aster-code-review"
note = ("> [!NOTE]\n"
        f"> These comments were generated by AI using the [`aster-code-review`]({url}) "
        "skill. Please take them seriously——but also with a grain of salt. "
        "You can run `aster-code-review` locally to iterate on your changes "
        "(review -> fix -> re-review) and catch issues before they reach manual review.")
body = note
for part in (os.environ["SUMMARY"], os.environ["DROPPED_SECTION"]):
    if part:
        body += "\n\n" + part
print(json.dumps({"commit_id": os.environ["HEAD_SHA"], "body": body, "comments": []}))')
RESP=$(echo "$PAYLOAD" | gh api "repos/$REPO/pulls/$PR/reviews" --method POST --input -) || {
    echo "error: failed to create review." >&2; echo "$RESP" >&2; exit 1; }
REVIEW_NODE_ID=$(echo "$RESP" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("node_id",""))')
REVIEW_ID=$(echo "$RESP"      | python3 -c 'import sys,json;print(json.load(sys.stdin).get("id",""))')
[ -n "$REVIEW_NODE_ID" ] || { echo "error: no review node_id in response." >&2; exit 1; }

ADDED=0 FAILED=0
if [ "$N_KEEP" -gt 0 ]; then
    while IFS= read -r C; do
        [ -n "$C" ] || continue
        QUERY=$(echo "$C" | REVIEW_NODE_ID="$REVIEW_NODE_ID" python3 -c '
import sys, json, os
c = json.load(sys.stdin); rid = os.environ["REVIEW_NODE_ID"]
body = json.dumps(c["body"]); path = json.dumps(c["path"])
if c.get("subject_type") == "file":
    inner = f"pullRequestReviewId: {json.dumps(rid)}, body: {body}, path: {path}, subjectType: FILE"
else:
    line = c["line"]; side = c["side"]
    inner = f"pullRequestReviewId: {json.dumps(rid)}, body: {body}, path: {path}, line: {line}, side: {side}"
    if c.get("start_line") and c["start_line"] != c["line"]:
        start_line = c["start_line"]; start_side = c["start_side"]
        inner += f", startLine: {start_line}, startSide: {start_side}"
print("mutation { addPullRequestReviewThread(input: {" + inner + "}) { thread { id } } }")')
        if gh api graphql -f query="$QUERY" >/dev/null 2>&1; then
            ADDED=$((ADDED + 1))
        else
            FAILED=$((FAILED + 1))
            echo "  warning: failed to add comment on $(echo "$C" | python3 -c 'import sys,json;print(json.load(sys.stdin)["path"])')"
        fi
    done < <(echo "$COMMENTS" | python3 -c 'import sys,json;[print(json.dumps(c)) for c in json.load(sys.stdin)]')
    echo "added $ADDED comment(s)$([ "$FAILED" -gt 0 ] && echo ", $FAILED failed")."
fi

# --- finalize or leave pending ---
if [ "$FINALIZE" = 1 ]; then
    gh api graphql -f query="mutation { submitPullRequestReview(input: { pullRequestReviewId: \"$REVIEW_NODE_ID\", event: $API_EVENT }) { pullRequestReview { state } } }" >/dev/null \
        && echo "review submitted ($EVENT): https://github.com/$REPO/pull/$PR" \
        || { echo "error: could not finalize; review left PENDING at https://github.com/$REPO/pull/$PR" >&2; exit 1; }
else
    echo "review left PENDING (review id $REVIEW_ID) — finalize on GitHub: https://github.com/$REPO/pull/$PR"
fi
