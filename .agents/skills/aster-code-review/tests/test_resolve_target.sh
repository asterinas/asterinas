#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Test cases for scripts/resolve_target.sh.
# Run via `make -C tests test_resolve_target`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
RT="$HERE/../scripts/resolve_target.sh"

setup() { build_repo "$TMP/repo"; FIX="$TMP/repo"; }
rt()      { ( cd "$FIX" && "$RT" "$@" ); }                          # capture stdout
rt_code() { ( cd "$FIX" && "$RT" "$@" ) >/dev/null 2>&1; echo $?; } # capture exit
field()   { sed -n "s/^$1=//p"; }                                   # pull a meta field from stdin

# --- argument grammar (each a distinct failure mode) -------------------------

test_requires_mode_word()         { assert_eq "no mode rejected"          "$(rt_code '')"                          2; }
test_rejects_unknown_mode()       { assert_eq "bad mode rejected"         "$(rt_code 'bogus a out.md')"            2; }
test_rejects_unknown_flag()       { assert_eq "unknown flag rejected"     "$(rt_code 'diff --bogus main out.md')"  2; }
test_needs_target_and_output()    { assert_eq "lone positional rejected"  "$(rt_code 'files out.md')"              2; }
test_rejects_unbalanced_quote()   { assert_eq "unbalanced quote rejected" "$(rt_code 'files "a out.md')"           2; }

# --- diff mode: grammar ------------------------------------------------------

test_diff_one_base_only()         { assert_eq "two bases rejected"   "$(rt_code 'diff main feature out.md')"   2; }
test_diff_rejects_range()         { assert_eq "base..head rejected"  "$(rt_code 'diff main..feature out.md')"  2; }
test_diff_rejects_lines_on_base() { assert_eq "base:lines rejected"  "$(rt_code 'diff main:1-2 out.md')"       2; }
# An empty commit range (base == HEAD) must fail closed, not emit an empty review.
test_diff_no_commits_is_error()   { assert_eq "empty base..HEAD rejected" "$(rt_code 'diff HEAD out.md')"      2; }

# --- diff mode: meta + canonical input (merge-base..HEAD commit series) -------

test_diff_meta_uses_merge_base() {
    local m mb; m="$(rt --meta 'diff main out.md')"
    mb="$(git -C "$FIX" rev-parse --short "$(git -C "$FIX" merge-base main HEAD)")"
    assert_eq "mode=diff"         "$(printf '%s\n' "$m" | field mode)"   "diff"
    assert_eq "base = merge-base" "$(printf '%s\n' "$m" | field base)"   "$mb"
    assert_eq "head = HEAD short" "$(printf '%s\n' "$m" | field head)"   "$(git -C "$FIX" rev-parse --short HEAD)"
    assert_eq "branch recorded"   "$(printf '%s\n' "$m" | field branch)" "feature"
    assert_eq "output = last pos" "$(printf '%s\n' "$m" | field output)" "out.md"
}

test_diff_input_is_commit_series() {
    local d; d="$(rt 'diff main out.md')"
    assert_contains "per-commit header"           "$d" "===== commit"
    assert_contains "includes the commit message" "$d" "F1"
    assert_contains "shows feature's new file"    "$d" "b/b.txt"
    assert_contains "shows feature's edit"        "$d" "+more"
    assert_absent   "excludes a main-only commit" "$d" "m.txt"
}

test_diff_excludes_uncommitted_edits() {
    printf 'base\nmore\nuncommitted\n' > "$FIX/a.txt"   # uncommitted (not a commit)
    assert_absent "uncommitted edit not in the commit series" "$(rt 'diff main out.md')" "+uncommitted"
    assert_absent "head not -dirty in diff mode" "$(rt --meta 'diff main out.md' | field head)" "-dirty"
}

# --- files mode: meta, ranges, excerpts --------------------------------------

test_files_meta_records_files_no_base() {
    local m; m="$(rt --meta 'files a.txt:5-8,1-2 b.txt out.md')"
    assert_eq "mode=files"                "$(printf '%s\n' "$m" | field mode)"   "files"
    assert_eq "files merged+sorted"       "$(printf '%s\n' "$m" | field files)"  "a.txt:1-2,5-8,b.txt"
    assert_eq "no base in files mode"     "$(printf '%s\n' "$m" | field base)"   ""
    assert_eq "output = last pos"         "$(printf '%s\n' "$m" | field output)" "out.md"
}

test_files_whole_file_excerpt() {
    local o; o="$(rt 'files a.txt out.md')"
    assert_contains "whole-file header" "$o" "===== a.txt ====="
    assert_contains "line 1 numbered"   "$o" "     1"$'\t'"base"
    assert_contains "line 2 numbered"   "$o" "     2"$'\t'"more"
}

test_files_line_range_excerpt() {
    local o; o="$(rt 'files a.txt:2-2 out.md')"
    assert_contains "range header"   "$o" "===== a.txt lines 2-2 ====="
    assert_contains "only line 2"    "$o" "     2"$'\t'"more"
    assert_absent   "excludes line 1" "$o" "     1"$'\t'"base"
}

test_files_missing_file_errors()  { assert_eq "missing file rejected" "$(rt_code 'files nope.txt out.md')" 2; }

# --- --per-persona-context flag ------------------------------------------------

test_app_defaults_to_auto()   { assert_eq "default is auto"   "$(rt --meta 'diff main out.md' | field per_persona_context)" "auto"; }
test_app_explicit_no()        { assert_eq "no recorded"       "$(rt --meta 'files a.txt --per-persona-context=no out.md' | field per_persona_context)" "no"; }
test_app_explicit_yes()       { assert_eq "yes recorded"      "$(rt --meta 'diff main --per-persona-context=yes out.md' | field per_persona_context)" "yes"; }
test_app_rejects_bad_value()  { assert_eq "bad value rejected" "$(rt_code 'diff main --per-persona-context=bogus out.md')" 2; }

run_suite
