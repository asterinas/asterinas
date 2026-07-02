#!/usr/bin/env bash
#
# post_reviews_to_github.sh — post an aster-code-review review file to a GitHub PR.
#
# The review file is GitHub-agnostic (the skill never knows about a PR), so the PR
# coordinates are arguments, not frontmatter:
#
#   post_reviews_to_github.sh --repo <owner/repo> --pr <N> --head-sha <sha> \
#                             [--finalize] [--event comment|approve|request_changes] \
#                             <review-file>
#
# What it does:
#   1. Parses the review file — the `# Summary` (+ any consolidated-fix blockquotes)
#      becomes the review body; each `### `path` line N` becomes an inline comment.
#   2. Drops comments whose line is not on the RIGHT side of the PR diff (GitHub
#      silently discards those), warning about each.
#   3. Creates a PENDING review (body only) and adds the comments via GraphQL
#      addPullRequestReviewThread (reliable line/side positioning).
#   4. With --finalize, submits the review as --event (default: comment). Without it,
#      the review is left PENDING for a human to submit on GitHub.
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
PARSED=$(REVIEW_FILE="$REVIEW_FILE" DIFF="$(gh pr diff "$PR" --repo "$REPO" 2>/dev/null || true)" python3 "$HERE/parse-review.py")

SUMMARY=$(echo "$PARSED"  | python3 -c 'import sys,json;print(json.load(sys.stdin)["summary"])')
N_KEEP=$(echo "$PARSED"   | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["comments"]))')
N_DROP=$(echo "$PARSED"   | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["dropped"]))')
COMMENTS=$(echo "$PARSED" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["comments"]))')

echo "parsed $N_KEEP inline comment(s) on diff lines; dropped $N_DROP off-diff."
if [ "$N_DROP" -gt 0 ]; then
    echo "$PARSED" | python3 -c '
import sys, json
for c in json.load(sys.stdin)["dropped"]:
    where = "file-level" if c.get("subject_type")=="file" else ("line "+str(c.get("line")))
    print(f"  dropped (not in PR diff): {c[\"path\"]} ({where})")'
fi
if [ "$N_KEEP" -eq 0 ] && [ -z "$SUMMARY" ]; then
    echo "nothing to post (no summary, no in-diff comments)."; exit 0
fi

# --- create a PENDING review (body only), then add comments as review threads ---
PAYLOAD=$(SUMMARY="$SUMMARY" HEAD_SHA="$HEAD_SHA" python3 -c '
import json, os
print(json.dumps({"commit_id": os.environ["HEAD_SHA"], "body": os.environ["SUMMARY"], "comments": []}))')
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
    inner = f"pullRequestReviewId: {json.dumps(rid)}, body: {body}, path: {path}, line: {c[\"line\"]}, side: {c[\"side\"]}"
    if c.get("start_line") and c["start_line"] != c["line"]:
        inner += f", startLine: {c[\"start_line\"]}, startSide: {c[\"start_side\"]}"
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
