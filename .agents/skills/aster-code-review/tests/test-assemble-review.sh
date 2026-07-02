#!/usr/bin/env bash
# Test cases for scripts/assemble-review.sh.
# Run via `make -C tests test-assemble-review`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
AS="$HERE/../scripts/assemble-review.sh"

setup() {
    FR="$TMP/frags"; mkdir -p "$FR"
    META="$TMP/meta"
    printf 'date=2026-06-25\nmode=diff\nbase=a1\nhead=9f3a1c2\nbranch=demo\ntitle=A "q" title\n' > "$META"
}
frag()  { printf '%s' "$2" > "$FR/$1.json"; }            # frag <persona> <json-array>
asm()   { "$AS" "$@" "$META" "$FR" "$TMP/out.md" 2>/dev/null; }   # asm [--overwrite]
out()   { cat "$TMP/out.md" 2>/dev/null; }
count() { grep -cF "$1" "$TMP/out.md" 2>/dev/null || true; }

C1='[{"file":"x.rs","line":5,"grounding":"bug","severity":"major","problem":"p1","fix":"do x","diff":"+x"}]'

# --- rendering ---------------------------------------------------------------

test_renders_comment_block() {
    frag security "$C1"; asm
    local o; o="$(out)"
    assert_contains "location heading"   "$o" '### `x.rs` line 5'
    assert_contains "grounding+severity" "$o" '`bug` (major): p1'
    assert_contains "fix paragraph"      "$o" '**Fix.** do x'
}

# --- grouping & ordering -----------------------------------------------------

test_groups_personas_in_fixed_order() {
    frag security "$C1"; frag maintainability "$C1"; asm
    assert_before "Maintainability before Security" "$(out)" "## Maintainability" "## Security"
}

test_sorts_comments_by_file_then_line() {
    frag development '[{"file":"x.rs","line":20,"grounding":"bug","severity":"minor","problem":"late","fix":"f","diff":"+l"},{"file":"x.rs","line":3,"grounding":"bug","severity":"minor","problem":"early","fix":"f","diff":"+e"}]'
    asm
    assert_before "line 3 before line 20" "$(out)" "line 3" "line 20"
}

# --- deduplication (the ADR-0003 behavior) -----------------------------------

test_dedups_identical_comments_within_a_persona() {
    local obj="${C1#[}"; obj="${obj%]}"        # the bare comment object, no [ ]
    frag development "[$obj, $obj]"            # two identical comments in one persona
    asm
    assert_eq "duplicate collapsed to one" "$(count '### `x.rs` line 5')" 1
}

test_keeps_same_comment_across_two_personas() {
    frag security "$C1"; frag maintainability "$C1"; asm
    assert_eq "kept in both persona sections" "$(count '### `x.rs` line 5')" 2
}

# --- frontmatter, overwrite guard, arity ------------------------------------

test_escapes_yaml_title() {
    frag security "$C1"; asm
    assert_contains "title quote-escaped" "$(out)" 'title: "A \"q\" title"'
}

test_refuses_to_overwrite_without_flag() {
    frag security "$C1"; asm            # first write creates out.md
    asm; assert_eq "second write refused" "$?" 1
}

test_overwrite_flag_replaces_existing() {
    frag security "$C1"; asm
    asm --overwrite; assert_eq "overwrite succeeds" "$?" 0
}

test_rejects_wrong_arity() {
    "$AS" "$META" "$FR" 2>/dev/null; assert_eq "missing output arg rejected" "$?" 2
}

# --- frontmatter records the review mode ------------------------------------

test_frontmatter_diff_mode() {
    frag security "$C1"; asm
    local o; o="$(out)"
    assert_contains "mode line" "$o" "mode: diff"
    assert_contains "base line" "$o" "base: a1"
    assert_absent   "no files line" "$o" "files:"
}

test_frontmatter_files_mode() {
    printf 'date=2026-06-25\nmode=files\nfiles=x.rs:1-5\nhead=9f3a1c2\nbranch=demo\n' > "$TMP/meta2"
    frag security "$C1"
    "$AS" "$TMP/meta2" "$FR" "$TMP/out2.md" 2>/dev/null
    local o; o="$(cat "$TMP/out2.md")"
    assert_contains "mode files" "$o" "mode: files"
    assert_contains "files line" "$o" "files: x.rs:1-5"
    assert_absent   "no base line" "$o" "base:"
}

run_suite
