#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Test cases for scripts/assemble_review.sh.
# Run via `make -C tests test_assemble_review`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
AS="$HERE/../scripts/assemble_review.sh"

setup() {
    FR="$TMP/frags"; mkdir -p "$FR"
    META="$TMP/meta"
    printf 'date=2026-06-25\nmode=diff\nbase=a1\nhead=9f3a1c2\nbranch=demo\ntitle=A "q" title\n' > "$META"
}
frag()  { printf '%s' "$2" > "$FR/$1.json"; }            # frag <persona> <json-array>
asm()   { "$AS" "$@" "$META" "$FR" "$TMP/out.md" 2>/dev/null; }   # asm [--overwrite]
out()   { cat "$TMP/out.md" 2>/dev/null; }
count() { grep -cF "$1" "$TMP/out.md" 2>/dev/null || true; }

C1='[{"file":"x.rs","line":5,"grounding":"lock-ordering","severity":"major","problem":"p1","fix":"do x","diff":"+x"}]'

# --- rendering ---------------------------------------------------------------

test_renders_comment_block() {
    frag security "$C1"; asm
    local o; o="$(out)"
    assert_contains "location heading"        "$o" '### `x.rs` line 5'
    assert_contains "guideline backticked"    "$o" '`lock-ordering` (major): p1'
    assert_contains "fix paragraph"           "$o" '**Fix.** do x'
}

test_bug_grounding_renders_as_plain_prose() {
    # A bug's grounding is a description, not a kebab identifier,
    # so it is NOT backticked
    # — it must never read as a guideline short-name.
    frag development '[{"file":"y.rs","line":9,"grounding":"Off by one","severity":"major","problem":"count wrong","fix":"f","diff":"+d"}]'
    asm
    local o; o="$(out)"
    assert_contains "description rendered as prose" "$o" 'Off by one (major): count wrong'
    assert_absent   "description not backticked"    "$o" '`Off by one`'
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

# --- deduplication (per-persona) ---------------------------------------------

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

test_keeps_comments_differing_in_any_field() {
    # Same file/line/problem/fix but different grounding+severity:
    # NOT exact duplicates, so dedup must keep both (a full-object key, not a partial one).
    frag development '[{"file":"x.rs","line":5,"grounding":"bug","severity":"major","problem":"p","fix":"f","diff":"+a"},{"file":"x.rs","line":5,"grounding":"lock-ordering","severity":"minor","problem":"p","fix":"f","diff":"+a"}]'
    asm
    assert_eq "both kept (differ in grounding/severity)" "$(count '### `x.rs` line 5')" 2
}

# --- fail closed on broken pass output (recall-first) ------------------------

test_fails_closed_on_unparseable_fragment() {
    frag development 'not valid json'
    asm; assert_eq "unparseable fragment aborts with exit 2" "$?" 2
}

test_fails_closed_on_non_array_fragment() {
    frag development '{"file":"x.rs"}'          # valid JSON, but an object not an array
    asm; assert_eq "non-array fragment aborts with exit 2" "$?" 2
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
