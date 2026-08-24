#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 TARGET_DIR" >&2
    exit 2
fi

TARGET_DIR=$1
LTP_PREBUILT_DIR=${LTP_PREBUILT_DIR:-/opt/ltp}
CONFORMANCE_TEST_WORKDIR=${CONFORMANCE_TEST_WORKDIR:-/tmp}
CONFORMANCE_TEST_SELECTOR=${CONFORMANCE_TEST_SELECTOR:-}

SCRIPT_DIR=$(dirname "$0")
ALL_TESTS="$SCRIPT_DIR/testcases/all.txt"
EXT2_BLOCKLIST="$SCRIPT_DIR/testcases/blocked/ext2.txt"
EXFAT_BLOCKLIST="$SCRIPT_DIR/testcases/blocked/exfat.txt"
RUN_BASH="$SCRIPT_DIR/run_ltp_test.sh"
SYSCALLS="$LTP_PREBUILT_DIR/runtest/syscalls"

REQUESTED_TESTS=$(mktemp)
SELECTED_TESTS=$(mktemp)

trap 'rm -f "$REQUESTED_TESTS" "$SELECTED_TESTS"' EXIT

if [ ! -r "$SYSCALLS" ]; then
    echo "Missing LTP syscall definitions: $SYSCALLS" >&2
    exit 1
fi

rm -rf "$TARGET_DIR"
mkdir -p "$TARGET_DIR/testcases/bin" "$TARGET_DIR/runtest"

filter_tests_by_blocklist() {
    blocklist=$1

    awk '
        FILENAME == ARGV[1] {
            if (!/^#/ && NF) {
                blocked[$0] = 1
            }
            next
        }
        !/^#/ && NF && !($0 in blocked)
    ' "$blocklist" "$ALL_TESTS" > "$SELECTED_TESTS"
}

if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
    printf '%s\n' "$CONFORMANCE_TEST_SELECTOR" | tr ',' '\n' > "$REQUESTED_TESTS"

    selected_count=0
    invalid_selector=0
    while IFS= read -r syscall || [ -n "$syscall" ]; do
        syscall=${syscall#"${syscall%%[![:space:]]*}"}
        syscall=${syscall%"${syscall##*[![:space:]]}"}
        [ -z "$syscall" ] && continue

        selected_count=$((selected_count + 1))
        binary=$(awk -v selected="$syscall" '$1 == selected { print $2; exit }' "$SYSCALLS")
        if [ -z "$binary" ]; then
            echo "Error: unknown LTP test: $syscall" >&2
            invalid_selector=1
            continue
        fi
        if [ ! -f "$LTP_PREBUILT_DIR/testcases/bin/$binary" ]; then
            echo "Error: LTP test binary is not available: $syscall ($binary)" >&2
            invalid_selector=1
            continue
        fi
        printf '%s\n' "$syscall" >> "$SELECTED_TESTS"
    done < "$REQUESTED_TESTS"

    if [ "$selected_count" -eq 0 ]; then
        echo "Error: CONFORMANCE_TEST_SELECTOR contains no test names" >&2
        exit 2
    fi
    if [ "$invalid_selector" -ne 0 ]; then
        exit 2
    fi
elif [ "$CONFORMANCE_TEST_WORKDIR" = "/ext2" ]; then
    filter_tests_by_blocklist "$EXT2_BLOCKLIST"
elif [ "$CONFORMANCE_TEST_WORKDIR" = "/exfat" ]; then
    filter_tests_by_blocklist "$EXFAT_BLOCKLIST"
else
    awk '!/^#/ && NF' "$ALL_TESTS" > "$SELECTED_TESTS"
fi

: > "$TARGET_DIR/runtest/syscalls"
while read -r syscall binary params; do
    if grep -qxF "$syscall" "$SELECTED_TESTS"; then
        if [ -f "$LTP_PREBUILT_DIR/testcases/bin/$binary" ]; then
            cp -f "$LTP_PREBUILT_DIR/testcases/bin/$binary" "$TARGET_DIR/testcases/bin"
            echo "$syscall $binary $params" >> "$TARGET_DIR/runtest/syscalls"
        else
            echo "Warning: $binary not found (skipping)"
        fi
    fi
done < "$SYSCALLS"

if [ -d "$LTP_PREBUILT_DIR/bin" ]; then
    cp -r "$LTP_PREBUILT_DIR/bin" "$TARGET_DIR"
fi
cp -r "$LTP_PREBUILT_DIR/libkirk" "$TARGET_DIR"
cp -f "$LTP_PREBUILT_DIR/kirk" "$TARGET_DIR"
cp -f "$LTP_PREBUILT_DIR/Version" "$TARGET_DIR"
cp -f "$LTP_PREBUILT_DIR/ver_linux" "$TARGET_DIR"
cp -f "$LTP_PREBUILT_DIR/IDcheck.sh" "$TARGET_DIR"
cp -f "$RUN_BASH" "$TARGET_DIR"
