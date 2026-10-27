#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Real libdevmapper calls, without disabling udev synchronization or substituting
# raw ioctls. Only tableless operations are in scope; no backing disk is touched.
set -eu
export LC_ALL=C

case "${1:-}" in
    dmsetup_version_and_major|dmsetup_tableless_lifecycle|dmsetup_identity_conflicts_and_minor_reuse)
        test_case=$1 ;;
    *) printf 'Unknown test case: %s\n' "${1:-}" >&2; exit 1 ;;
esac

work=$(mktemp -d /tmp/dm-management.XXXXXXXX)
prefix=${work##*/}
first=${prefix}-first
second=${prefix}-second
retry=${prefix}-retry
uuid=${prefix}-uuid
owned_names=

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

# Preserve stdout for structured assertions and show both streams on failure.
run() {
    printf 'CMD:'
    printf ' %s' "$@"
    printf '\n'
    if timeout -k 2s 20s "$@" >"$work/output" 2>"$work/error"; then
        command_status=0
    else
        command_status=$?
    fi
    cat "$work/output"
    cat "$work/error" >&2
    printf 'EXIT: %s\n' "$command_status"
    return "$command_status"
}

expect_failure() {
    if run "$@"; then
        fail "command unexpectedly succeeded: $*"
    else
        failure_status=$?
        # Do not mistake timeout, signal death, or missing executables for the
        # requested dmsetup rejection. dmsetup reports command failure as 1.
        [ "$failure_status" -eq 1 ] || fail "unexpected failure status $failure_status: $*"
    fi
}

cleanup() {
    original_status=$?
    trap - EXIT HUP INT TERM
    set +e
    cleanup_failed=0
    for cleanup_name in $owned_names; do
        timeout -k 2s 20s dmsetup info "$cleanup_name" >"$work/output" 2>"$work/error"
        lookup_status=$?
        if [ "$lookup_status" -eq 0 ]; then
            if ! timeout -k 2s 20s dmsetup remove "$cleanup_name" >"$work/output" 2>"$work/error"; then
                printf 'Cleanup failed: %s\n' "$cleanup_name" >&2
                cat "$work/output" "$work/error" >&2
                cleanup_failed=1
            fi
        elif [ "$lookup_status" -ne 1 ]; then
            printf 'Cleanup lookup failed (%s): %s\n' "$lookup_status" "$cleanup_name" >&2
            cat "$work/output" "$work/error" >&2
            cleanup_failed=1
        fi
    done
    if [ "$original_status" -eq 0 ] && [ "$cleanup_failed" -eq 0 ]; then
        rm -f "$work/output" "$work/error"
        rmdir "$work" || exit 1
        printf 'CASE_PASS_%s\n' "$test_case"
        exit 0
    fi
    printf 'CASE_FAIL_%s (logs: %s)\n' "$test_case" "$work" >&2
    exit 1
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# Fail before taking ownership if any generated name already exists. Register
# cleanup before CREATE: a userspace failure may follow a committed kernel ioctl.
prepare_names() {
    for candidate in "$first" "$second" "$retry"; do
        expect_failure dmsetup info "$candidate"
    done
    owned_names="$first $second $retry"
}

check_major() {
    [ -c /dev/mapper/control ] || fail 'missing control device'
    run cat /proc/devices
    registered_major=$(awk '
        /^Block devices:$/ { block = 1; next }
        block && $2 == "device-mapper" { print $1; count++ }
        END { if (count != 1) exit 1 }
    ' "$work/output") || fail 'expected exactly one device-mapper block major'
    case "$registered_major" in
        ''|*[!0-9]*) fail 'invalid registered block major' ;;
    esac
}

# Limit columns to identity and tableless state, avoiding table/dependency ioctls.
info_columns() {
    run dmsetup info -c --noheadings --separator '|' \
        -o name,uuid,major,minor,open,segments,events "$@"
    info=$(awk '{ gsub(/[[:space:]]/, ""); print }' "$work/output")
}

capture_identity() {
    info_columns "$first"
    initial=$info
    assigned_major=$(printf '%s\n' "$initial" | awk -F'|' '{ print $3 }')
    assigned_minor=$(printf '%s\n' "$initial" | awk -F'|' '{ print $4 }')
    [ "$assigned_major" = "$registered_major" ] || fail 'info major differs from /proc/devices'
    case "$assigned_minor" in
        ''|*[!0-9]*) fail 'invalid minor in info output' ;;
    esac
    [ "$initial" = "$first|$uuid|$assigned_major|$assigned_minor|0|0|0" ] \
        || fail "unexpected tableless state: $initial"
}

check_listed() {
    run dmsetup ls
    awk -v name="$first" -v major="$assigned_major" -v minor="$assigned_minor" '
        { gsub(/[[:space:]]/, "") }
        $0 == name "(" major ":" minor ")" { count++ }
        END { exit count != 1 }
    ' "$work/output" || fail 'list is missing the exact name and device number'
}

check_absent() {
    expect_failure dmsetup info "$first"
    expect_failure dmsetup info -u "$uuid"
    run dmsetup ls
    awk -v name="$first" '$1 == name { found = 1 } END { exit found ? 1 : 0 }' \
        "$work/output" || fail 'removed identity remains listed'
}

version_and_major() {
    check_major
    run dmsetup version
    grep -Eq '^Library version:[[:space:]]+[^[:space:]]' "$work/output" \
        || fail 'missing libdevmapper version'
    grep -Eq '^Driver version:[[:space:]]+4\.48\.0([[:space:]]|$)' "$work/output" \
        || fail 'unexpected kernel DM interface version'
}

tableless_lifecycle() {
    check_major
    prepare_names
    run dmsetup create "$first" --notable --uuid "$uuid"
    capture_identity

    run dmsetup info "$first"
    grep -Eq "^Name:[[:space:]]+$first$" "$work/output" || fail 'plain info name mismatch'
    info_columns -u "$uuid"
    [ "$info" = "$initial" ] || fail 'UUID lookup returned a different identity'
    info_columns -j "$assigned_major" -m "$assigned_minor"
    [ "$info" = "$initial" ] || fail 'device-number lookup returned a different identity'
    info_columns --inactive "$first"
    [ "$info" = "$initial" ] || fail 'inactive query changed tableless state'
    check_listed

    # libdevmapper may maintain nodes itself; their existence is not evidence of
    # kernel runtime publication, nor is it a failure of tableless management.
    run dmsetup remove "$first"
    check_absent

    # Exercise the no-UUID identity path as well as deletion selected by UUID.
    run dmsetup create "$second" --notable
    info_columns "$second"
    case "$info" in
        "$second||$registered_major|"*) ;;
        *) fail 'unexpected identity for creation without UUID' ;;
    esac
    run dmsetup remove "$second"
    run dmsetup create "$first" --notable --uuid "$uuid"
    run dmsetup remove -u "$uuid"
    check_absent
}

identity_conflicts_and_minor_reuse() {
    check_major
    prepare_names
    run dmsetup create "$first" --notable --uuid "$uuid"
    capture_identity

    expect_failure dmsetup create "$first" --notable
    info_columns "$first"
    [ "$info" = "$initial" ] || fail 'name collision changed the original identity'
    expect_failure dmsetup create "$second" --notable --uuid "$uuid"
    expect_failure dmsetup info "$second"
    info_columns -u "$uuid"
    [ "$info" = "$initial" ] || fail 'UUID collision changed the original identity'

    expect_failure dmsetup create "$second" --notable \
        --major "$assigned_major" --minor "$assigned_minor"
    expect_failure dmsetup info "$second"
    info_columns "$first"
    [ "$info" = "$initial" ] || fail 'minor collision changed the original identity'
    check_listed

    run dmsetup remove "$first"
    check_absent
    run dmsetup create "$retry" --notable --uuid "$uuid" \
        --major "$assigned_major" --minor "$assigned_minor"
    info_columns "$retry"
    [ "$info" = "$retry|$uuid|$assigned_major|$assigned_minor|0|0|0" ] \
        || fail 'removed UUID or minor was not reusable'
    run dmsetup remove "$retry"
    expect_failure dmsetup info "$retry"
    expect_failure dmsetup info -u "$uuid"
}

case "$test_case" in
    dmsetup_version_and_major) version_and_major ;;
    dmsetup_tableless_lifecycle) tableless_lifecycle ;;
    dmsetup_identity_conflicts_and_minor_reuse) identity_conflicts_and_minor_reuse ;;
esac
