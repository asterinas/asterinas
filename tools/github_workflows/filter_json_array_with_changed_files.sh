#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

print_help() {
    cat <<'EOF'
Usage: filter_json_array_with_changed_files.sh --array <json> [options]
       < <changed-files>

Reads changed file paths from stdin (one per line) and prints a filtered
version of the input JSON array, keeping only items whose subdirectories
contain at least one changed file. Workflow-neutral: emits its result to
stdout, no side effects.

Selection policy:
  For each changed file under <per-item-path-prefix>/<name>/**, keep the
  array item whose `name` field equals <name>. Items not selected by any
  changed file are dropped. Empty stdin or no matches -> [].

Required:
  --array <json>              JSON array of objects.

Optional:
  --per-item-path-prefix <path>
                              Base directory; a change under
                              <path>/<name>/** keeps the array item
                              whose `name` field equals <name>.
                              Repeatable.

  --name-field <key>          Field used to identify items by name.
                              Default: 'name'.

  -h, --help                  Print this help.

Inputs:
  Changed files on stdin, one per line. Empty stdin -> empty array.

Output:
  Filtered JSON array on stdout. Nothing else.

Exit status:
  0 on success (including empty result); non-zero on usage error.

Examples:

  # Change scoped to one item:
  $ echo test/nixos/tests/foo/src/main.rs \
      | filter_json_array_with_changed_files.sh \
        --array '[{"name":"foo"},{"name":"bar"}]' \
        --per-item-path-prefix 'test/nixos/tests'
  [{"name":"foo"}]

  # No matching change:
  $ echo README.md | filter_json_array_with_changed_files.sh \
        --array '[{"name":"foo"},{"name":"bar"}]' \
        --per-item-path-prefix 'test/nixos/tests'
  []
EOF
}

die() {
    echo "Error: $1" >&2
    exit 1
}

normalize_prefix() {
    local prefix="$1"
    printf '%s\n' "${prefix%/}"
}

filter_array_by_names() {
    local selected_names_json="$1"

    jq -c \
        --arg name_field "${name_field}" \
        --argjson selected_names "${selected_names_json}" \
        '
        [.[] | select((.[$name_field]) as $name | $selected_names | index($name))]
        ' <<<"${array_json}"
}

array_json=""
name_field="name"
per_item_path_prefixes=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --array)
            [ "$#" -ge 2 ] || die "--array requires an argument."
            array_json="$2"
            shift 2
            ;;
        --per-item-path-prefix)
            [ "$#" -ge 2 ] || die "--per-item-path-prefix requires an argument."
            per_item_path_prefixes+=("$(normalize_prefix "$2")")
            shift 2
            ;;
        --name-field)
            [ "$#" -ge 2 ] || die "--name-field requires an argument."
            name_field="$2"
            shift 2
            ;;
        -h|--help)
            print_help
            exit 0
            ;;
        *)
            die "unknown option '$1'."
            ;;
    esac
done

[ -n "${array_json}" ] || {
    print_help >&2
    die "--array is required."
}

array_json="$(
    jq -c '
        if type == "array" then
            .
        else
            error("input must be a JSON array")
        end
    ' <<<"${array_json}"
)"

declare -A selected_names=()
changed_file=""
prefix=""

while IFS= read -r changed_file; do
    [ -n "${changed_file}" ] || continue

    for prefix in "${per_item_path_prefixes[@]}"; do
        if [[ "${changed_file}" != "${prefix}/"* ]]; then
            continue
        fi

        selected_name="${changed_file#"${prefix}/"}"
        if [[ "${selected_name}" != */* ]]; then
            continue
        fi
        selected_name="${selected_name%%/*}"
        [ -n "${selected_name}" ] || continue
        selected_names["${selected_name}"]=1
    done
done

selected_names_json="$(
    if [ "${#selected_names[@]}" -eq 0 ]; then
        printf '[]\n'
    else
        printf '%s\n' "${!selected_names[@]}" | jq -Rsc '
            split("\n")
            | map(select(length > 0))
            | sort
        '
    fi
)"

filter_array_by_names "${selected_names_json}"
