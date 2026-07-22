#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Test cases for scripts/guideline_query.py.
# Run via `make -C tests test_guideline_query`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
QUERY="$HERE/../scripts/guideline_query.py"
OVERLAY="$HERE/../benchmark/overlay_skill.sh"
REPO="$(cd "$HERE/../../../.." && pwd)"

query() { python3 "$QUERY" "$@"; }
query_code() { python3 "$QUERY" "$@" >/dev/null 2>&1; echo $?; }
catalog_digest() {
    local header
    header="$(query catalog "$1")"; header="${header%%$'\n'*}"
    header="${header#*digest=}"
    printf '%s\n' "${header%% *}"
}
query_show() {
    local persona="$1"; shift
    query show --expect-digest "$(catalog_digest "$persona")" "$persona" "$@"
}

write_fixture() {
    local root="$1" index="$2"
    local dir="$root/book/src/to-contribute/coding-guidelines/for-security"
    mkdir -p "$root"
    cp -r "$REPO/book" "$root/book"
    rm -rf "$dir"
    mkdir -p "$dir"
    printf '%s\n' \
        '# For Security' \
        '' \
        '## Index' \
        '' \
        "$index" > "$dir/README.md"
    cat > "$dir/rules.md" <<'EOF'
# Rules

### Alpha rule (`alpha`) {#alpha}

Alpha body.

```md
```not-a-closing-fence
### This fenced heading is an example, not a rule (`fake`) {#fake}
```

### Beta rule (`beta`) {#beta}

Beta body.
EOF
}

test_catalog_contains_gists_but_not_rule_bodies() {
    local out
    out="$(query catalog development)"
    assert_contains "catalog header" "$out" "GUIDELINE_CATALOG persona=development rules=18"
    assert_contains "catalog gist" "$out" 'Use checked or saturating arithmetic where overflow is possible'
    assert_absent "detail body omitted" "$out" 'Overflow is often a correctness and security issue'
}

test_show_extracts_one_rule_without_its_neighbor() {
    local out
    out="$(query_show development checked-arithmetic)"
    assert_contains "selected rule heading" "$out" '### Use checked or saturating arithmetic (`checked-arithmetic`)'
    assert_contains "selected rule source" "$out" 'correctness.md#checked-arithmetic'
    assert_absent "neighbor omitted" "$out" '### Use `debug_assert` for correctness-only checks'
}

test_show_batches_in_catalog_order_and_deduplicates() {
    local out checked_count checked_pos propagated_pos
    out="$(query_show development propagate-errors checked-arithmetic checked-arithmetic)"
    checked_count="$(printf '%s\n' "$out" | rg -c '^--- rule: checked-arithmetic ---$')"
    checked_pos="${out%%'--- rule: checked-arithmetic ---'*}"
    propagated_pos="${out%%'--- rule: propagate-errors ---'*}"
    assert_eq "duplicate requested rule emitted once" "$checked_count" 1
    [[ ${#checked_pos} -lt ${#propagated_pos} ]] || {
        _fail=$((_fail + 1)); _note "rules are not in catalog order"
    }
}

test_show_rejects_unknown_and_cross_persona_ids() {
    local digest
    digest="$(catalog_digest development)"
    assert_eq "unknown rule rejected" \
        "$(query_code show --expect-digest "$digest" development no-such-rule)" 2
    assert_eq "cross-persona rule rejected" \
        "$(query_code show --expect-digest "$digest" development explain-why)" 2
}

test_show_requires_matching_catalog_digest_when_provided() {
    local header digest out
    header="$(query catalog development)"; header="${header%%$'\n'*}"
    digest="${header#*digest=}"; digest="${digest%% *}"
    out="$(query show --expect-digest "$digest" development checked-arithmetic)"
    assert_contains "matching digest accepted" "$out" 'ids=checked-arithmetic'
    assert_eq "mismatched digest rejected" \
        "$(query_code show --expect-digest "$(printf '0%.0s' {1..64})" development checked-arithmetic)" 2
}

test_show_rejects_missing_catalog_digest() {
    assert_eq "show requires catalog digest" \
        "$(query_code show development checked-arithmetic)" 2
}

test_check_validates_all_personas() {
    local out
    out="$(query check)"
    assert_contains "all personas checked" "$out" 'personas=maintainability,development,security,hardware,documentation'
    assert_contains "all rules counted" "$out" 'rules=77'
}

test_check_rejects_orphan_rule() {
    local root="$TMP/root"
    write_fixture "$root" '- [`alpha`](rules.md#alpha): Alpha gist.'
    assert_eq "orphan detail rule rejected" \
        "$(ACR_GUIDELINE_ROOT="$root" query_code check security)" 2
}

test_check_rejects_malformed_rule_heading() {
    local root="$TMP/root" rules
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`beta`](rules.md#beta): Beta gist.'
    rules="$root/book/src/to-contribute/coding-guidelines/for-security/rules.md"
    printf '\n## Wrong-level rule (`gamma`) {#gamma}\n\nMalformed body.\n' >> "$rules"
    assert_eq "rule-shaped non-H3 rejected" \
        "$(ACR_GUIDELINE_ROOT="$root" query_code check security)" 2
}

test_check_rejects_malformed_rule_like_index_item() {
    local root="$TMP/root" readme
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`beta`](rules.md#beta): Beta gist.'
    readme="$root/book/src/to-contribute/coding-guidelines/for-security/README.md"
    printf '%s\n' '- [`broken`](rules.md#broken)' >> "$readme"
    assert_eq "malformed rule-like index item rejected" \
        "$(ACR_GUIDELINE_ROOT="$root" query_code check security)" 2
}

test_catalog_rejects_partial_corpus() {
    local root="$TMP/root"
    mkdir -p "$root"
    cp -r "$REPO/book" "$root/book"
    rm -rf "$root/book/src/to-contribute/coding-guidelines/for-documentation"
    assert_eq "catalog requires complete corpus" \
        "$(ACR_GUIDELINE_ROOT="$root" query_code catalog development)" 2
}

test_fenced_heading_does_not_end_a_rule_chunk() {
    local root="$TMP/root" out
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`beta`](rules.md#beta): Beta gist.'
    out="$(ACR_GUIDELINE_ROOT="$root" query_show security alpha)"
    assert_contains "fenced example remains in alpha" "$out" 'This fenced heading is an example, not a rule'
    assert_absent "next real rule omitted" "$out" 'Beta body.'
}

test_root_precedence_prefers_explicit_then_bundled_then_repo() {
    local wt="$TMP/wt" stale="$TMP/stale" bundled explicit fallback
    mkdir -p "$wt" "$stale"
    "$OVERLAY" "$wt"
    cp -r "$REPO/book" "$stale/book"
    sed -i 's/Use checked or saturating arithmetic where overflow is possible/EXPLICIT_ROOT_SENTINEL/' \
        "$stale/book/src/to-contribute/coding-guidelines/for-development/README.md"

    bundled="$(python3 "$wt/.agents/skills/aster-code-review/scripts/guideline_query.py" catalog development)"
    explicit="$(ACR_GUIDELINE_ROOT="$stale" \
        python3 "$wt/.agents/skills/aster-code-review/scripts/guideline_query.py" catalog development)"
    fallback="$(query catalog development)"

    assert_absent "bundled root hides explicit-only sentinel" "$bundled" 'EXPLICIT_ROOT_SENTINEL'
    assert_contains "explicit root has highest priority" "$explicit" 'EXPLICIT_ROOT_SENTINEL'
    assert_contains "normal repo fallback works" "$fallback" 'Use checked or saturating arithmetic where overflow is possible'
}

test_overlay_requires_bundled_snapshot() {
    local wt="$TMP/wt"
    mkdir -p "$wt"
    "$OVERLAY" "$wt"
    cp -r "$REPO/book" "$wt/book"
    rm -rf "$wt/.agents/skills/aster-code-review/guideline-root"
    assert_eq "overlay refuses historical worktree fallback" \
        "$(python3 "$wt/.agents/skills/aster-code-review/scripts/guideline_query.py" root >/dev/null 2>&1; echo $?)" 2
}

test_root_command_reports_resolved_root() {
    local root="$TMP/root" out
    mkdir -p "$root"
    cp -r "$REPO/book" "$root/book"
    out="$(ACR_GUIDELINE_ROOT="$root" query root)"
    assert_eq "root command uses shared resolver" "$out" "$root"
}

test_stats_reports_catalog_detail_and_rule_counts() {
    local out
    out="$(query stats maintainability)"
    assert_contains "stats persona" "$out" '"persona": "maintainability"'
    assert_contains "stats rules" "$out" '"rules": 44'
    assert_contains "stats catalog bytes" "$out" '"catalog_bytes": 7479'
    assert_contains "stats detail bytes" "$out" '"detail_bytes": 31354'
}

run_suite
