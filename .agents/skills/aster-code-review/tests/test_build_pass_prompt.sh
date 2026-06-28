#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Test cases for scripts/build_pass_prompt.sh.
# Run via `make -C tests test_build_pass_prompt`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
BP="$HERE/../scripts/build_pass_prompt.sh"

setup() {
    IN="$TMP/in1.txt";  printf 'REVIEW_SENTINEL_ONE\nsome diff\n' > "$IN"
    IN2="$TMP/in2.txt"; printf 'a completely different second input\n' > "$IN2"
}
bp()      { "$BP" "$@"; }
bp_code() { "$BP" "$@" >/dev/null 2>&1; echo $?; }
# Everything up to and including the REVIEW INPUT marker — the cache-stable prefix.
prefix()  { sed '/^===== REVIEW INPUT =====$/q'; }

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

test_inlines_guideline_page() {
    assert_contains "dev guideline inlined" "$(bp "$IN" development)" "for-development/README.md"
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
    # The benchmark overrides the guideline root;
    # the inlined page must come from it.
    local g="$TMP/groot"
    mkdir -p "$g/book/src/to-contribute/coding-guidelines/for-development"
    echo "SENTINEL_GUIDELINE_OVERRIDE" > "$g/book/src/to-contribute/coding-guidelines/for-development/README.md"
    assert_contains "guideline read from ACR_GUIDELINE_ROOT" \
        "$(ACR_GUIDELINE_ROOT="$g" bp "$IN" development)" "SENTINEL_GUIDELINE_OVERRIDE"
}

run_suite
