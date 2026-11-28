#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

upstream_dir=/opt/apparmor/upstream
sys_profiles=/sys/kernel/security/apparmor/profiles
parser_conf=/etc/apparmor/parser.conf
empty_md5=d41d8cd98f00b204e9800998ecf8427e

fail()
{
    echo "APPARMOR_TEST_ERROR: $*" >&2
    exit 1
}

check_dummy_absent()
{
    local profiles
    local grep_status

    profiles=$(cat "$sys_profiles") ||
        fail "cannot read $sys_profiles"

    if grep -Eq '^dummy_test \([^)]+\)$' <<< "$profiles"; then
        fail "dummy_test is still present in profiles"
    else
        # Exit 1 means no match; other nonzero statuses are errors.
        grep_status=$?
        [ "$grep_status" -eq 1 ] ||
            fail "cannot check profiles"
    fi
}

for tool in awk bash cat cp env grep hexdump md5sum mktemp sed tail wc zstdcat; do
    command -v "$tool" >/dev/null ||
        fail "required tool was not found: $tool"
done

[ -r "$sys_profiles" ] ||
    fail "profiles is not readable: $sys_profiles"

[ -r "$parser_conf" ] ||
    fail "parser.conf is not readable: $parser_conf"

[ -x /bin/true ] ||
    fail "/bin/true is not executable"

[ -x /bin/pwd ] ||
    fail "/bin/pwd is not executable"

[ -x /usr/bin/perl ] ||
    fail "/usr/bin/perl is not executable"

parser=$(command -v apparmor_parser) ||
    fail "apparmor_parser was not found"

[ -x "$parser" ] ||
    fail "apparmor_parser is not executable: $parser"

# Avoid removing a profile that existed before this test.
check_dummy_absent

aa_tests=$(mktemp -d) ||
    fail "cannot create temporary directory"

trap 'rm -rf "$aa_tests"' EXIT

cp -R "$upstream_dir/." "$aa_tests/" ||
    fail "cannot copy upstream regression files"

cd "$aa_tests" ||
    fail "cannot enter temporary directory"

# These variables cover e2e.sh; other upstream tests may need more.
printf 'subdomain=%q\nparser_config=%q\ntmpdir=%q\nsys_profiles=%q\n' \
    "$parser" \
    "--config-file=$parser_conf" \
    "/tmp/apparmor-upstream-e2e" \
    "$sys_profiles" \
    > uservars.inc

echo "APPARMOR_PRECHECK_OK"

run_round()
{
    local round=$1
    local trace_file="$aa_tests/round${round}.trace"
    local effective_cache="$aa_tests/round${round}.cache"

    local cache_dir cache_file magic cache_bytes
    local cache_hash_line cache_hash traced_kernel_hash

    # Stabilize the trace and avoid inheriting exported shell options.
    if env -u SHELLOPTS PS4='+ ' bash -x ./e2e.sh 2>"$trace_file"; then
        cat "$trace_file"
    else
        cat "$trace_file" >&2
        fail "round $round: upstream e2e failed"
    fi

    echo "APPARMOR_ROUND${round}_UPSTREAM_OK"

    check_dummy_absent
    echo "APPARMOR_ROUND${round}_LIST_CLEAR_OK"

    cache_dir=$("$parser" --print-cache-dir) ||
        fail "round $round: cannot locate parser cache"
    cache_file="$cache_dir/profile"

    [ -r "$cache_file" ] ||
        fail "round $round: cache file is not readable: $cache_file"

    # Match e2e.sh's cache decoding before checking its bytes and hash.
    magic=$(hexdump -s 8 -n 4 --format '"%x"' "$cache_file") ||
        fail "round $round: cannot inspect cache header"

    if [ "$magic" = "fd2fb528" ]; then
        tail -c +9 "$cache_file" | zstdcat > "$effective_cache" ||
            fail "round $round: cannot decompress cache"
    else
        cp "$cache_file" "$effective_cache" ||
            fail "round $round: cannot copy cache"
    fi

    cache_bytes=$(wc -c < "$effective_cache") ||
        fail "round $round: cannot count cache bytes"
    [ "$cache_bytes" -gt 0 ] ||
        fail "round $round: effective cache is empty"

    cache_hash_line=$(md5sum "$effective_cache") ||
        fail "round $round: cannot hash effective cache"
    cache_hash=${cache_hash_line%% *}

    traced_kernel_hash=$(
        sed -n 's/^+ kernel_md5=//p' "$trace_file" | tail -n 1
    )

    [[ "$traced_kernel_hash" =~ ^[0-9a-f]{32}$ ]] ||
        fail "round $round: kernel hash missing or unexpected in trace"
    [ "$cache_hash" = "$traced_kernel_hash" ] ||
        fail "round $round: effective cache and kernel raw_data hashes differ"
    [ "$traced_kernel_hash" != "$empty_md5" ] ||
        fail "round $round: kernel raw_data appears empty"

    echo "APPARMOR_ROUND${round}_CACHE_BYTES=$cache_bytes"
    echo "APPARMOR_ROUND${round}_CACHE_MD5=$cache_hash"
    echo "APPARMOR_ROUND${round}_RAW_DATA_MD5=$traced_kernel_hash"
    echo "APPARMOR_ROUND${round}_DATA_OK"
}

run_round 1
run_round 2

echo "APPARMOR_UPSTREAM_CONTROL_PLANE_OK"
