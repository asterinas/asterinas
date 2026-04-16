#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

PROJECT_ROOT="$(realpath "$(dirname "${BASH_SOURCE[0]}")/..")"
WORKSPACE_MANIFEST="${PROJECT_ROOT}/Cargo.toml"

usage() {
    cat <<'EOF'
Usage:
  ./tools/print_workspace_members.sh [--default-ones | --non-default-ones] [--package-names]

Print the relative paths of Cargo workspace members.

Options:
  --default-ones      Only print `default-members`.
  --non-default-ones  Only print workspace members outside `default-members`.
  --package-names     Print package names rather than paths.
  -h, --help          Show this help message.
EOF
}

ensure_command() {
    local command_name="$1"

    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Error: required command '${command_name}' is not installed or not in PATH." >&2
        exit 1
    fi
}

workspace_excludes_json() {
    python3 - "$WORKSPACE_MANIFEST" <<'PY'
import json
import sys
import tomllib

with open(sys.argv[1], "rb") as manifest_file:
    manifest = tomllib.load(manifest_file)

print(json.dumps(manifest.get("workspace", {}).get("exclude", [])))
PY
}

print_workspace_members() {
    local member_filter="$1"
    local output_kind="$2"
    local jq_program

    jq_program='
        def manifest_dir:
            .manifest_path
            | if startswith($project_root + "/") then
                .[($project_root | length) + 1:]
              else
                .
              end
            | sub("/Cargo.toml$"; "");

        . as $metadata
        | ($metadata.packages | map({key: .id, value: .}) | from_entries) as $packages
        | ($metadata.workspace_default_members // []) as $default_members
        | (
            if $member_filter == "all" then
                $metadata.workspace_members
            elif $member_filter == "default" then
                $default_members
            elif $member_filter == "non-default" then
                $metadata.workspace_members
                | map(select(. as $member | ($default_members | index($member) | not)))
            else
                error("unknown member filter")
            end
          )
        | .[]
        | $packages[.]
        | { name, dir: manifest_dir }
        | select(.dir as $dir | ($workspace_excludes | index($dir) | not))
        | if $output_kind == "dirs" then
            .dir
          elif $output_kind == "package-names" then
            .name
          else
            error("unknown output kind")
          end
    '

    (
        cd "$PROJECT_ROOT"
        cargo metadata --format-version 1 --no-deps \
            | jq -r \
                --arg member_filter "$member_filter" \
                --arg output_kind "$output_kind" \
                --arg project_root "$PROJECT_ROOT" \
                --argjson workspace_excludes "$(workspace_excludes_json)" \
                "$jq_program"
    )
}

main() {
    local member_filter="all"
    local output_kind="dirs"

    while (($# > 0)); do
        case "$1" in
            --default-ones)
                if [[ "$member_filter" != "all" ]]; then
                    echo "Error: only one workspace-member filter can be specified." >&2
                    usage >&2
                    exit 1
                fi
                member_filter="default"
                ;;
            --non-default-ones)
                if [[ "$member_filter" != "all" ]]; then
                    echo "Error: only one workspace-member filter can be specified." >&2
                    usage >&2
                    exit 1
                fi
                member_filter="non-default"
                ;;
            --package-names)
                output_kind="package-names"
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                echo "Error: unknown option '$1'." >&2
                usage >&2
                exit 1
                ;;
        esac

        shift
    done

    ensure_command cargo
    ensure_command jq
    ensure_command python3

    print_workspace_members "$member_filter" "$output_kind"
}

main "$@"
