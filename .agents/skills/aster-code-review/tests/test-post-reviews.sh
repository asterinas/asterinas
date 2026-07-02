#!/usr/bin/env bash
# Tests for scripts/parse-review.py — the review-file parser behind
# post_reviews_to_github.sh.
# Model-free: exercises summary extraction, comment parsing, and the in-diff
# filter directly, with no gh and no network.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
PARSER="$HERE/../scripts/parse-review.py"

setup() {
    cat > "$TMP/review.md" <<'EOF'
---
date: 2026-07-02
mode: diff
base: abc123
head: def456
branch: test
title: "Test change"
---

# Summary

This change does X and Y.
There is one load-bearing issue.

> **Consolidated fix C1.** Restore the removed guard.

---

## Security

### `src/foo.rs` line 10

> ```diff
> -    bad
> +    good
> ```

`justify-unsafe-use` (critical): The problem at line 10.

**Fix.** Shared fix C1.

### `src/foo.rs` lines 20-22

> ```diff
> +a
> +b
> ```

`bug` (major): A multi-line problem.

**Fix.** Do something.

## Maintainability

### `src/bar.rs` line 99

`descriptive-names` (minor): This line is not in the diff.

**Fix.** Rename it.

### `src/foo.rs`

`readme-as-crate-doc` (nit): A file-level note.

**Fix.** Add docs.
EOF
    cat > "$TMP/pr.diff" <<'EOF'
diff --git a/src/foo.rs b/src/foo.rs
index 1111111..2222222 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -8,3 +8,4 @@ fn f() {
 line8
 line9
-    bad
+    good
+    line11
@@ -19,1 +20,3 @@ fn g() {
+a
+b
+c
EOF
}

# run parser; arg "no-diff" => empty DIFF (nothing is filtered out)
_parse() {
    if [[ "${1:-}" == no-diff ]]; then
        REVIEW_FILE="$TMP/review.md" DIFF="" python3 "$PARSER"
    else
        REVIEW_FILE="$TMP/review.md" DIFF="$(cat "$TMP/pr.diff")" python3 "$PARSER"
    fi
}
_f() { echo "$1" | python3 -c "import sys,json;d=json.load(sys.stdin);print($2)"; }

test_summary_is_summary_plus_consolidated_fix() {
    local out s; out="$(_parse)"; s="$(_f "$out" 'd["summary"]')"
    assert_contains "summary keeps prose"            "$s" "This change does X and Y."
    assert_contains "summary keeps consolidated fix"  "$s" "Consolidated fix C1"
    assert_absent   "summary stops before personas"   "$s" "## Security"
    assert_absent   "summary drops trailing rule"      "$s" "---"
}

test_in_diff_kept_offdiff_dropped() {
    local out; out="$(_parse)"
    assert_eq "3 in-diff comments kept"     "$(_f "$out" 'len(d["comments"])')" 3
    assert_eq "1 off-diff comment dropped"  "$(_f "$out" 'len(d["dropped"])')"  1
    assert_eq "dropped one is bar.rs"       "$(_f "$out" 'd["dropped"][0]["path"]')" "src/bar.rs"
}

test_line_comment_fields() {
    local out; out="$(_parse)"
    assert_eq       "line number"       "$(_f "$out" 'd["comments"][0]["line"]')" 10
    assert_eq       "RIGHT side"        "$(_f "$out" 'd["comments"][0]["side"]')" "RIGHT"
    assert_contains "body has grounding" "$(_f "$out" 'd["comments"][0]["body"]')" "justify-unsafe-use"
    assert_absent   "body excludes diff" "$(_f "$out" 'd["comments"][0]["body"]')" "good"
}

test_range_comment_fields() {
    local out; out="$(_parse)"
    assert_eq "range end line"   "$(_f "$out" 'd["comments"][1]["line"]')"       22
    assert_eq "range start line" "$(_f "$out" 'd["comments"][1]["start_line"]')" 20
}

test_file_level_comment() {
    local out; out="$(_parse)"
    assert_eq "file-level subject_type" "$(_f "$out" 'd["comments"][2].get("subject_type","")')" "file"
}

test_no_diff_keeps_everything() {
    local out; out="$(_parse no-diff)"
    assert_eq "no diff -> nothing dropped" "$(_f "$out" 'len(d["dropped"])')"  0
    assert_eq "no diff -> all four kept"   "$(_f "$out" 'len(d["comments"])')" 4
}

run_suite
