#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

PROJECT_ROOT="$(realpath "$(dirname "${BASH_SOURCE[0]}")/..")"
WORKSPACE_MANIFEST="${PROJECT_ROOT}/Cargo.toml"
LINUX_BZIMAGE_SETUP_DIR="ostd/libs/linux-bzimage/setup"

usage() {
    cat <<'EOF'
Usage:
  ./tools/workspace_members.sh <member-set> <output-kind>

Member sets:
  members      Cargo workspace members, excluding `workspace.exclude` entries
  default      Cargo workspace `default-members`
  non-default  Workspace members that are neither `default-members` nor
               `ostd/libs/linux-bzimage/setup`

Output kinds:
  dirs          Print one crate directory per line
  package-args  Print repeated `-p <package>` arguments, one shell argument
                per line
EOF
}

ensure_command() {
    local command_name="$1"

    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Error: required command '${command_name}' is not installed or not in PATH." >&2
        exit 1
    fi
}

print_workspace_members() {
    local member_set="$1"
    local output_kind="$2"
    local jq_program
    local workspace_excludes_json

    ensure_command cargo
    ensure_command jq
    ensure_command tomlq

    workspace_excludes_json="$(tomlq -c '.workspace.exclude // []' "$WORKSPACE_MANIFEST")"
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
            if $member_set == "members" then
                $metadata.workspace_members
            elif $member_set == "default" then
                $default_members
            elif $member_set == "non-default" then
                $metadata.workspace_members
                | map(select(. as $member | ($default_members | index($member) | not)))
            else
                error("unknown member set")
            end
          )
        | .[]
        | $packages[.]
        | { name, dir: manifest_dir }
        | select(.dir as $dir | ($workspace_excludes | index($dir) | not))
        | select($member_set != "non-default" or .dir != $linux_bzimage_setup_dir)
        | if $output_kind == "dirs" then
            .dir
          elif $output_kind == "package-args" then
            "-p", .name
          else
            error("unknown output kind")
          end
    '

    (
        cd "$PROJECT_ROOT"
        cargo metadata --format-version 1 --no-deps \
            | jq -r \
                --arg member_set "$member_set" \
                --arg output_kind "$output_kind" \
                --arg project_root "$PROJECT_ROOT" \
                --argjson workspace_excludes "$workspace_excludes_json" \
                --arg linux_bzimage_setup_dir "$LINUX_BZIMAGE_SETUP_DIR" \
                "$jq_program"
    )
}

main() {
    local member_set="${1:-}"
    local output_kind="${2:-}"

    if [[ -z "$member_set" || -z "$output_kind" ]]; then
        usage >&2
        exit 1
    fi

    case "$member_set" in
        members|default|non-default)
            ;;
        *)
            echo "Error: unknown member set '${member_set}'." >&2
            usage >&2
            exit 1
            ;;
    esac

    case "$output_kind" in
        dirs|package-args)
            print_workspace_members "$member_set" "$output_kind"
            ;;
        *)
            echo "Error: unknown output kind '${output_kind}'." >&2
            usage >&2
            exit 1
            ;;
    esac
}

main "$@"
