#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Tests for scripts/parse_review.py
# — the review-file parser behind post_reviews_to_github.sh.
# Model-free:
# exercises summary extraction, comment parsing, and the in-diff filter directly,
# with no gh and no network.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
PARSER="$HERE/../scripts/parse_review.py"

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

test_dropped_section_surfaces_offdiff_findings() {
    local out sec; out="$(_parse)"; sec="$(_f "$out" 'd["dropped_section"]')"
    assert_contains "section has heading"          "$sec" "Findings not attachable to this PR's diff"
    assert_contains "section explains provenance"   "$sec" "[!NOTE]"
    assert_contains "section hedges pre-existing"   "$sec" "pre-existing"
    assert_contains "section locates the finding"   "$sec" "\`src/bar.rs\` line 99"
    assert_contains "section keeps the problem"     "$sec" "not in the diff"
    assert_contains "section keeps the fix"         "$sec" "Rename it"
}

test_no_dropped_section_when_nothing_dropped() {
    local out; out="$(_parse no-diff)"
    assert_eq "empty section when nothing dropped" "$(_f "$out" 'd["dropped_section"]')" ""
}

# parse a custom review + diff pair (both as here-strings written to $TMP)
_parse_rd() { REVIEW_FILE="$1" DIFF="$(cat "$2")" python3 "$PARSER"; }

# Regression: inter-file `diff --git`/`index` lines must NOT be counted as context
# for the previous file (else a comment just past a file's last hunk is wrongly kept).
test_multifile_no_phantom_lines_after_last_hunk() {
    cat > "$TMP/r2.md" <<'EOF'
# Summary

x

## Correctness

### `a.rs` line 4

`bug-x` (major): just past a.rs's last hunk — not a real diff line.

**Fix.** none.

### `b.rs` line 2

`bug-y` (major): a real added line in b.rs.

**Fix.** none.
EOF
    cat > "$TMP/d2.diff" <<'EOF'
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,3 @@
 ctx1
+added2
 ctx3
diff --git a/b.rs b/b.rs
index 333..444 100644
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,2 @@
 bctx1
+badded2
EOF
    local out; out="$(_parse_rd "$TMP/r2.md" "$TMP/d2.diff")"
    assert_eq       "a.rs:4 (phantom) dropped" "$(_f "$out" 'len(d["dropped"])')" 1
    assert_eq       "dropped is a.rs"          "$(_f "$out" 'd["dropped"][0]["path"]')" "a.rs"
    assert_eq       "b.rs:2 kept"              "$(_f "$out" 'len(d["comments"])')" 1
    assert_eq       "kept is b.rs"             "$(_f "$out" 'd["comments"][0]["path"]')" "b.rs"
}

# Regression: an added line whose own text starts with `+++ ` must be treated as
# content, not a file header (else the rest of the hunk is lost from `postable`).
test_added_line_starting_with_plus_is_content_not_header() {
    cat > "$TMP/r3.md" <<'EOF'
# Summary

x

## Correctness

### `c.rs` line 3

`bug-z` (major): a real added line AFTER a `+++`-looking added line.

**Fix.** none.
EOF
    cat > "$TMP/d3.diff" <<'EOF'
diff --git a/c.rs b/c.rs
index 1..2 100644
--- a/c.rs
+++ b/c.rs
@@ -1,1 +1,3 @@
 ctx1
+++ this added line literally starts with plus-plus-plus
+realadd
EOF
    local out; out="$(_parse_rd "$TMP/r3.md" "$TMP/d3.diff")"
    assert_eq "c.rs:3 kept (hunk not truncated)" "$(_f "$out" 'len(d["comments"])')" 1
    assert_eq "nothing dropped"                  "$(_f "$out" 'len(d["dropped"])')" 0
}

# Fix: a dropped RANGE finding surfaces its start line, not just the end line.
test_dropped_range_shows_start_line() {
    cat > "$TMP/r4.md" <<'EOF'
# Summary

x

## Correctness

### `z.rs` lines 20-22

`bug-r` (major): a multi-line finding on a file not in the diff.

**Fix.** none.
EOF
    cat > "$TMP/d4.diff" <<'EOF'
diff --git a/other.rs b/other.rs
index 1..2 100644
--- a/other.rs
+++ b/other.rs
@@ -1,1 +1,2 @@
 keep
+add
EOF
    local out sec; out="$(_parse_rd "$TMP/r4.md" "$TMP/d4.diff")"; sec="$(_f "$out" 'd["dropped_section"]')"
    assert_contains "range shows start-end" "$sec" "\`z.rs\` lines 20-22"
}

run_suite
