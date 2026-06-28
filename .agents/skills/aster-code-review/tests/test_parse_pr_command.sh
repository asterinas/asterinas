#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Tests for scripts/parse_pr_command.sh
# — the strict allowlist parser for /aster-code-review PR-comment commands.
# Model-free.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
P="$HERE/../scripts/parse_pr_command.sh"

run() { OUT="$("$P" "$1" 2>/dev/null)"; RC=$?; }   # sets $OUT, $RC

# --- valid commands ---------------------------------------------------------
test_bare_is_diff() {
    run "/aster-code-review"
    assert_eq       "accepted"     "$RC" 0
    assert_contains "kind review"  "$OUT" "kind=review"
    assert_contains "mode diff"    "$OUT" "mode=diff"
}
test_explicit_diff() {
    run "/aster-code-review diff"
    assert_eq "accepted" "$RC" 0
    assert_contains "mode diff" "$OUT" "mode=diff"
}
test_files_paths() {
    run "/aster-code-review files a.rs sub/dir/b.rs"
    assert_eq       "accepted"    "$RC" 0
    assert_contains "mode files"  "$OUT" "mode=files"
    assert_contains "paths kept"  "$OUT" "paths=a.rs sub/dir/b.rs"
}
test_smoke_bare() {
    run "/aster-code-review smoke"
    assert_eq       "accepted"       "$RC" 0
    assert_contains "kind test"      "$OUT" "kind=test"
    assert_contains "target smoke"   "$OUT" "target=smoke"
    assert_contains "problems empty" "$OUT" "problems="
}
test_benchmark_with_problems() {
    run '/aster-code-review benchmark --problems="0002 0006"'
    assert_eq       "accepted"        "$RC" 0
    assert_contains "target bench"    "$OUT" "target=benchmark"
    assert_contains "problems parsed" "$OUT" "problems=0002 0006"
}
test_problems_without_quotes() {
    run "/aster-code-review smoke --problems=0002"
    assert_eq       "accepted"      "$RC" 0
    assert_contains "problems 0002" "$OUT" "problems=0002"
}
test_trigger_on_later_line_with_indent() {
    run "$(printf 'hey please\n  /aster-code-review smoke\nthanks')"
    assert_eq       "accepted"     "$RC" 0
    assert_contains "target smoke" "$OUT" "target=smoke"
}

# --- rejected: unknown / malformed -----------------------------------------
test_reject_unknown_subcommand() { run "/aster-code-review frobnicate"; assert_eq "rejected" "$RC" 2; }
test_reject_diff_with_args()     { run "/aster-code-review diff main";  assert_eq "rejected" "$RC" 2; }
test_reject_no_command()         { run "just a normal comment";        assert_eq "rejected" "$RC" 2; }
test_reject_trigger_substring()  { run "/aster-code-reviewX smoke";     assert_eq "rejected" "$RC" 2; }

# --- rejected: injection attempts (the whole point) ------------------------
test_reject_path_traversal()   { run "/aster-code-review files ../etc/passwd";     assert_eq "rejected" "$RC" 2; }
test_reject_absolute_path()    { run "/aster-code-review files /etc/passwd";       assert_eq "rejected" "$RC" 2; }
test_reject_path_metachars()   { run '/aster-code-review files a;rm';              assert_eq "rejected" "$RC" 2; }
test_reject_path_subshell()    { run '/aster-code-review files $(rm)';             assert_eq "rejected" "$RC" 2; }
test_reject_problems_inject()  { run '/aster-code-review benchmark --problems="; rm -rf /"'; assert_eq "rejected" "$RC" 2; }
test_reject_nonproblems_flag() { run "/aster-code-review benchmark --keep=1";      assert_eq "rejected" "$RC" 2; }
test_reject_problems_letters() { run '/aster-code-review smoke --problems="abc"';  assert_eq "rejected" "$RC" 2; }
# The old make-style knob is no longer the comment syntax;
# only --problems= is.
test_reject_make_style_knob()  { run '/aster-code-review smoke PROBLEMS="0002"';   assert_eq "rejected" "$RC" 2; }

run_suite
