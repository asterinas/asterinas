#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SYSFS_DIR="/sys/devices/virtual/misc/tdx_guest/measurements"

# Check that a file is readable and returns exactly SHA384_DIGEST_SIZE (48) bytes.
check_readable() {
    name="$1"
    path="$SYSFS_DIR/$name"
    size=$(wc -c < "$path")
    if [ "$size" -ne 48 ]; then
        echo "FAIL: $name returned $size bytes, expected 48" >&2
        exit 1
    fi
    echo "PASS: $name is readable (48 bytes)"
}

# Check that a file is readable and contains at least one non-zero byte.
check_readable_nonzero() {
    name="$1"
    path="$SYSFS_DIR/$name"
    # od -A n suppresses the address column, leaving only hex data bytes.
    # Stripping spaces/newlines and then all '0' digits leaves a non-empty
    # string iff at least one byte is non-zero.
    nz=$(od -A n -t x1 < "$path" | tr -d ' \n')
    if [ -z "$nz" ]; then
        echo "FAIL: $name is empty" >&2
        exit 1
    fi
    cleaned=$(echo "$nz" | tr -d '0')
    if [ -z "$cleaned" ]; then
        echo "FAIL: $name read back as all-zeros" >&2
        exit 1
    fi
    echo "PASS: $name is readable and non-zero"
}

# Verify that writing to a read-only register is rejected (EACCES / EPERM).
check_write_rejected() {
    name="$1"
    path="$SYSFS_DIR/$name"
    if dd if=/dev/urandom bs=48 count=1 > "$path" 2>/dev/null; then
        echo "FAIL: write to read-only register $name was not rejected" >&2
        exit 1
    fi
    echo "PASS: write to read-only register $name was correctly rejected"
}

# Verify that writing a wrong-sized payload to an RTMR is rejected.
check_wrong_size_rejected() {
    name="$1"
    size="$2"
    path="$SYSFS_DIR/$name"
    if dd if=/dev/urandom bs="$size" count=1 > "$path" 2>/dev/null; then
        echo "FAIL: write of $size bytes to $name was not rejected" >&2
        exit 1
    fi
    echo "PASS: write of $size bytes to $name was correctly rejected"
}

# --- 1. Read-only registers: smoke-test readability --------------------------

echo "=== Read-only register smoke tests ==="
# mrconfigid, mrowner, mrownerconfig may be all-zeros when not configured; only verify readability.
for name in mrconfigid mrowner mrownerconfig; do
    check_readable "$name"
done
# mrtd is set by the TDX module at TD creation time and must be non-zero.
check_readable_nonzero "mrtd:sha384"

# --- 2. RTMR extend: verify value changes after write ------------------------

echo "=== RTMR extend before/after tests ==="
i=0
while [ $i -le 3 ]; do
    name="rtmr${i}:sha384"
    path="$SYSFS_DIR/$name"
    echo "Testing RTMR${i}..."

    before=$(hd "$path")
    dd if=/dev/urandom bs=48 count=1 > "$path" 2>/dev/null
    after=$(hd "$path")

    if [ "$before" = "$after" ]; then
        echo "FAIL: RTMR${i} value did not change after extend" >&2
        exit 1
    fi
    echo "PASS: RTMR${i} value changed after extend"
    i=$((i + 1))
done

# --- 3. Error paths ----------------------------------------------------------

echo "=== Error path tests ==="

# Writing to a read-only register must be rejected.
for name in mrconfigid mrowner mrownerconfig "mrtd:sha384"; do
    check_write_rejected "$name"
done

# Writing wrong-sized data to an RTMR must be rejected.
for size in 32 64; do
    i=0
    while [ $i -le 3 ]; do
        check_wrong_size_rejected "rtmr${i}:sha384" "$size"
        i=$((i + 1))
    done
done

echo "=== All TSM-MR tests completed successfully ==="
