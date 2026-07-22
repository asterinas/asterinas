#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Test cases for scripts/build_pass_prompt.sh.
# Run via `make -C tests test_build_pass_prompt`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
BP="$HERE/../scripts/build_pass_prompt.sh"
REPO="$(cd "$HERE/../../../.." && pwd)"

setup() {
    IN="$TMP/in1.txt";  printf 'REVIEW_SENTINEL_ONE\nsome diff\n' > "$IN"
    IN2="$TMP/in2.txt"; printf 'a completely different second input\n' > "$IN2"
}
bp()      { "$BP" "$@"; }
bp_code() { "$BP" "$@" >/dev/null 2>&1; echo $?; }
# Everything up to and including the REVIEW INPUT marker — the cache-stable prefix.
# Consume the complete stream so the prompt builder never sees a broken pipe.
prefix()  { awk '!done { print } /^===== REVIEW INPUT =====$/ { done = 1 }'; }

# --- arity / validation ------------------------------------------------------

test_requires_input()           { assert_eq "no args rejected"        "$(bp_code)"                 2; }
test_requires_persona()         { assert_eq "input but no persona"    "$(bp_code "$IN")"           2; }
test_rejects_unknown_persona()  { assert_eq "bad persona rejected"    "$(bp_code "$IN" bogus)"     2; }
test_rejects_missing_input()    { assert_eq "missing input file"      "$(bp_code /no/such development)" 2; }

# --- ordering: stable head (contract -> persona -> guideline) then input -----

test_orders_contract_persona_input() {
    local o; o="$(bp "$IN" development)"
    assert_before "contract before persona"   "$o" "Pass contract"            "PERSONA: development"
    assert_before "persona before input"      "$o" "PERSONA: development"      "===== REVIEW INPUT ====="
    assert_before "input marker before body"  "$o" "===== REVIEW INPUT =====" "REVIEW_SENTINEL_ONE"
}

test_progressive_inlines_catalog_but_not_rule_body() {
    local o
    o="$(bp "$IN" development)"
    assert_contains "catalog header" "$o" "GUIDELINE_CATALOG persona=development rules=18"
    assert_contains "catalog gist" "$o" "Use checked or saturating arithmetic where overflow is possible"
    assert_absent "detail body deferred" "$o" "Prefer explicit overflow handling"
}

test_progressive_includes_simple_print_command() {
    local o
    o="$(bp "$IN" development)"
    assert_contains "print command" "$o" ".agents/skills/aster-code-review/scripts/print_guidelines.py"
    assert_contains "persona precedes short-names" "$o" '<persona> <short-name>...'
    assert_absent "no required digest argument" "$o" "--expect-digest"
}

test_full_mode_inlines_rule_body() {
    assert_contains "full mode includes detail" \
        "$(ACR_GUIDELINE_DISCLOSURE=full bp "$IN" development)" \
        "Prefer explicit overflow handling"
}

test_full_mode_rejects_missing_persona_corpus() {
    local g="$TMP/groot"
    mkdir -p "$g/book/src/to-contribute/coding-guidelines"
    assert_eq "full mode fails closed without persona corpus" \
        "$(ACR_GUIDELINE_ROOT="$g" ACR_GUIDELINE_DISCLOSURE=full bp_code "$IN" development)" 2
}

test_full_mode_does_not_depend_on_progressive_parser() {
    local g="$TMP/groot" rules out
    mkdir -p "$g"
    cp -r "$REPO/book" "$g/book"
    rules="$g/book/src/to-contribute/coding-guidelines/for-development/correctness.md"
    printf '\n### Rollback-only prose heading\n\nFULL_MODE_SENTINEL\n' >> "$rules"
    out="$(ACR_GUIDELINE_ROOT="$g" ACR_GUIDELINE_DISCLOSURE=full bp "$IN" development)"
    assert_contains "full mode emits pages rejected by progressive parser" "$out" "FULL_MODE_SENTINEL"
}

test_rejects_unknown_disclosure_mode() {
    assert_eq "unknown disclosure mode rejected" \
        "$(ACR_GUIDELINE_DISCLOSURE=bogus bp_code "$IN" development)" 2
}

# --- the cache property: stable prefix is byte-identical regardless of input --

test_stable_prefix_is_input_independent() {
    local a b
    a="$(bp "$IN"  development | prefix)"
    b="$(bp "$IN2" development | prefix)"
    assert_eq "stable prefix identical across inputs" "$a" "$b"
}

test_input_body_absent_from_prefix() {
    assert_absent "input body not in the cached prefix" "$(bp "$IN" development | prefix)" "REVIEW_SENTINEL_ONE"
}

# --- combined mode: all personas, in order -----------------------------------

test_combined_lists_all_personas_in_order() {
    local o; o="$(bp "$IN" development security)"
    assert_before "development before security" "$o" "PERSONA: development" "PERSONA: security"
    assert_contains "security block present"    "$o" "PERSONA: security"
}

test_honours_acr_guideline_root() {
    local g="$TMP/groot"
    mkdir -p "$g"
    cp -r "$REPO/book" "$g/book"
    sed -i 's/Use checked or saturating arithmetic where overflow is possible/SENTINEL_GUIDELINE_OVERRIDE/' \
        "$g/book/src/to-contribute/coding-guidelines/for-development/README.md"
    assert_contains "guideline read from ACR_GUIDELINE_ROOT" \
        "$(ACR_GUIDELINE_ROOT="$g" bp "$IN" development)" "SENTINEL_GUIDELINE_OVERRIDE"
}

test_builder_reads_only_the_selected_persona_catalog() {
    local g="$TMP/groot"
    mkdir -p "$g"
    cp -r "$REPO/book" "$g/book"
    rm -rf "$g/book/src/to-contribute/coding-guidelines/for-documentation"
    assert_contains "unrelated missing persona does not block prompt" \
        "$(ACR_GUIDELINE_ROOT="$g" bp "$IN" development)" "PERSONA: development"
}

test_combined_progressive_omits_detail_pages() {
    local o
    o="$(bp "$IN" maintainability development security hardware documentation)"
    assert_contains "maintainability catalog present" "$o" "GUIDELINE_CATALOG persona=maintainability rules=44"
    assert_contains "documentation catalog present" "$o" "GUIDELINE_CATALOG persona=documentation rules=3"
    assert_absent "combined detail deferred" "$o" "Prefer explicit overflow handling"
}

test_progressive_prefix_stays_within_byte_budgets() {
    local maintainability_bytes combined_bytes
    maintainability_bytes="$(bp "$IN" maintainability | prefix | wc -c)"
    combined_bytes="$(bp "$IN" maintainability development security hardware documentation | prefix | wc -c)"
    [[ $maintainability_bytes -le $((18 * 1024)) ]] || {
        _fail=$((_fail + 1)); _note "maintainability prefix is $maintainability_bytes bytes, budget is 18432"
    }
    [[ $combined_bytes -le $((32 * 1024)) ]] || {
        _fail=$((_fail + 1)); _note "combined prefix is $combined_bytes bytes, budget is 32768"
    }
}

run_suite
