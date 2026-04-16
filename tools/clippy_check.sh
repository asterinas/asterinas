#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

PROJECT_ROOT="$(realpath "$(dirname "${BASH_SOURCE[0]}")/..")"
LINUX_BZIMAGE_SETUP_DIR="ostd/libs/linux-bzimage/setup"

usage() {
    cat <<'EOF'
Usage:
  ./tools/clippy_check.sh osdk
      Runs `cargo clippy --all-targets --no-deps` for the standalone `osdk`
      crate.

  [OSDK_TARGET_ARCH=x86_64|riscv64|loongarch64] \
  ./tools/clippy_check.sh workspace
      Runs the workspace clippy checks used by `make check`.
      This checks:
        - workspace `default-members` with `cargo osdk clippy`
        - non-default workspace members with `cargo clippy --all-targets`
        - `ostd/libs/linux-bzimage/setup` separately on `x86_64`

Options:
  -h, --help
      Shows this help message.
EOF
}

ensure_command() {
    local command_name="$1"

    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Error: required command '${command_name}' is not installed or not in PATH." >&2
        exit 1
    fi
}

build_package_args() {
    local member_filter="$1"
    local package_names
    local -n output_ref="$2"

    output_ref=()
    package_names="$("${PROJECT_ROOT}/tools/print_workspace_members.sh" "$member_filter" --package-names)"
    if [[ -z "$package_names" ]]; then
        return
    fi

    while IFS= read -r package_name; do
        output_ref+=(-p "$package_name")
    done <<< "$package_names"
}

run_check_osdk() {
    ensure_command cargo

    echo "Checking osdk"
    (
        cd "$PROJECT_ROOT/osdk"
        RUSTFLAGS="-Dwarnings" cargo clippy --all-targets --no-deps
    )
}

run_workspace_clippy() {
    local -a non_default_package_args=()
    local -a filtered_non_default_package_args=()

    ensure_command cargo
    ensure_command "${PROJECT_ROOT}/tools/print_workspace_members.sh"

    echo "Checking default workspace members"
    (
        cd "$PROJECT_ROOT"
        RUSTFLAGS="-Dwarnings" cargo osdk clippy -- --no-deps
        RUSTFLAGS="-Dwarnings" cargo osdk clippy --ktests -- --no-deps
    )

    build_package_args "--non-default-ones" non_default_package_args
    for ((index = 0; index < ${#non_default_package_args[@]}; index += 2)); do
        if [[ "${non_default_package_args[index + 1]}" = "linux-bzimage-setup" ]]; then
            continue
        fi

        filtered_non_default_package_args+=(
            "${non_default_package_args[index]}"
            "${non_default_package_args[index + 1]}"
        )
    done

    if ((${#filtered_non_default_package_args[@]} > 0)); then
        echo "Checking non-default workspace members"
        (
            cd "$PROJECT_ROOT"
            RUSTFLAGS="-Dwarnings" cargo clippy "${filtered_non_default_package_args[@]}" --all-targets --no-deps
        )
    fi

    # `linux-bzimage/setup` only supports x86_64 currently.
    # It may fail on other architectures.
    if [[ "${OSDK_TARGET_ARCH:-x86_64}" = "x86_64" ]]; then
        echo "Checking ${LINUX_BZIMAGE_SETUP_DIR}"
        (
            cd "$PROJECT_ROOT/$LINUX_BZIMAGE_SETUP_DIR"
            RUSTFLAGS="-Dwarnings" cargo osdk clippy -- --no-deps
            RUSTFLAGS="-Dwarnings" cargo osdk clippy --ktests -- --no-deps
        )
    fi
}

main() {
    local mode="${1:-}"

    case "$mode" in
        -h|--help)
            usage
            ;;
        osdk)
            run_check_osdk
            ;;
        workspace)
            run_workspace_clippy
            ;;
        *)
            usage >&2
            exit 1
            ;;
    esac
}

main "$@"
