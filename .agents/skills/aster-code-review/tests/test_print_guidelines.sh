#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Test cases for scripts/print_guidelines.py.
# Run via `make -C tests test_print_guidelines`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
PRINT_GUIDELINES="$HERE/../scripts/print_guidelines.py"
OVERLAY="$HERE/../benchmark/overlay_skill.sh"
REPO="$(cd "$HERE/../../../.." && pwd)"

print_guidelines() { python3 "$PRINT_GUIDELINES" "$@"; }
print_code() { python3 "$PRINT_GUIDELINES" "$@" >/dev/null 2>&1; echo $?; }

write_fixture() {
    local root="$1" index="$2"
    local dir="$root/book/src/to-contribute/coding-guidelines/for-security"
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
    out="$(print_guidelines development --catalog)"
    assert_contains "catalog header" "$out" "GUIDELINE_CATALOG persona=development rules=18"
    assert_contains "catalog gist" "$out" 'Use checked or saturating arithmetic where overflow is possible'
    assert_absent "detail body omitted" "$out" 'Overflow is often a correctness and security issue'
}

test_prints_one_guideline_without_its_neighbor() {
    local out
    out="$(print_guidelines development checked-arithmetic)"
    assert_contains "selected rule heading" "$out" '### Use checked or saturating arithmetic (`checked-arithmetic`)'
    assert_contains "selected rule source" "$out" 'correctness.md#checked-arithmetic'
    assert_absent "neighbor omitted" "$out" '### Use `debug_assert` for correctness-only checks'
}

test_prints_multiple_guidelines_in_catalog_order_and_deduplicates() {
    local out checked_count checked_pos propagated_pos
    out="$(print_guidelines development propagate-errors checked-arithmetic checked-arithmetic)"
    checked_count="$(printf '%s\n' "$out" | rg -c '^--- guideline: checked-arithmetic ---$')"
    checked_pos="${out%%'--- guideline: checked-arithmetic ---'*}"
    propagated_pos="${out%%'--- guideline: propagate-errors ---'*}"
    assert_eq "duplicate requested rule emitted once" "$checked_count" 1
    [[ ${#checked_pos} -lt ${#propagated_pos} ]] || {
        _fail=$((_fail + 1)); _note "guidelines are not in catalog order"
    }
}

test_omitting_short_names_prints_every_indexed_guideline() {
    local root="$TMP/root" out
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`beta`](rules.md#beta): Beta gist.'
    out="$(ACR_GUIDELINE_ROOT="$root" print_guidelines security)"
    assert_contains "first guideline emitted" "$out" '--- guideline: alpha ---'
    assert_contains "second guideline emitted" "$out" '--- guideline: beta ---'
}

test_rejects_unknown_and_cross_persona_short_names() {
    assert_eq "unknown guideline rejected" \
        "$(print_code development no-such-rule)" 2
    assert_eq "cross-persona guideline rejected" \
        "$(print_code development explain-why)" 2
}

test_catalog_rejects_short_names() {
    assert_eq "catalog and short-name are mutually exclusive" \
        "$(print_code development checked-arithmetic --catalog)" 2
}

test_catalog_rejects_malformed_index_item() {
    local root="$TMP/root" readme
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`beta`](rules.md#beta): Beta gist.'
    readme="$root/book/src/to-contribute/coding-guidelines/for-security/README.md"
    printf '%s\n' '- [`broken`](rules.md#broken)' >> "$readme"
    assert_eq "malformed index item rejected" \
        "$(ACR_GUIDELINE_ROOT="$root" print_code security --catalog)" 2
}

test_reads_only_the_requested_guideline_pages() {
    local root="$TMP/root" dir out
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`broken`](broken.md#broken): Broken gist.'
    dir="$root/book/src/to-contribute/coding-guidelines/for-security"
    printf '%s\n' '# Broken' '' '### Not a guideline heading' > "$dir/broken.md"

    out="$(ACR_GUIDELINE_ROOT="$root" print_guidelines security --catalog)"
    assert_contains "catalog does not parse detail pages" "$out" 'Broken gist.'
    out="$(ACR_GUIDELINE_ROOT="$root" print_guidelines security alpha)"
    assert_contains "unrelated malformed page is not read" "$out" 'Alpha body.'
    assert_eq "requested malformed page is rejected" \
        "$(ACR_GUIDELINE_ROOT="$root" print_code security broken)" 2
}

test_rejects_wrong_level_rule_heading_on_selected_page() {
    local root="$TMP/root" rules
    write_fixture "$root" '- [`alpha`](rules.md#alpha): Alpha gist.'
    rules="$root/book/src/to-contribute/coding-guidelines/for-security/rules.md"
    sed -i 's/^### Alpha rule/## Alpha rule/' "$rules"
    assert_eq "selected non-H3 rule rejected" \
        "$(ACR_GUIDELINE_ROOT="$root" print_code security alpha)" 2
}

test_fenced_heading_does_not_end_a_guideline() {
    local root="$TMP/root" out
    write_fixture "$root" $'- [`alpha`](rules.md#alpha): Alpha gist.\n- [`beta`](rules.md#beta): Beta gist.'
    out="$(ACR_GUIDELINE_ROOT="$root" print_guidelines security alpha)"
    assert_contains "fenced example remains in alpha" "$out" 'This fenced heading is an example, not a rule'
    assert_absent "next real rule omitted" "$out" 'Beta body.'
}

test_selected_persona_does_not_require_other_personas() {
    local root="$TMP/root" out
    mkdir -p "$root"
    cp -r "$REPO/book" "$root/book"
    rm -rf "$root/book/src/to-contribute/coding-guidelines/for-documentation"
    out="$(ACR_GUIDELINE_ROOT="$root" print_guidelines development --catalog)"
    assert_contains "selected persona catalog still works" "$out" 'persona=development'
}

test_root_precedence_prefers_explicit_then_bundled_then_repo() {
    local wt="$TMP/wt" stale="$TMP/stale" bundled explicit fallback
    mkdir -p "$wt" "$stale"
    "$OVERLAY" "$wt"
    cp -r "$REPO/book" "$stale/book"
    sed -i 's/Use checked or saturating arithmetic where overflow is possible/EXPLICIT_ROOT_SENTINEL/' \
        "$stale/book/src/to-contribute/coding-guidelines/for-development/README.md"

    bundled="$(python3 "$wt/.agents/skills/aster-code-review/scripts/print_guidelines.py" development --catalog)"
    explicit="$(ACR_GUIDELINE_ROOT="$stale" \
        python3 "$wt/.agents/skills/aster-code-review/scripts/print_guidelines.py" development --catalog)"
    fallback="$(print_guidelines development --catalog)"

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
        "$(python3 "$wt/.agents/skills/aster-code-review/scripts/print_guidelines.py" development --catalog >/dev/null 2>&1; echo $?)" 2
}

run_suite
