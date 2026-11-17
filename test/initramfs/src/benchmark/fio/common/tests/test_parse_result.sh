#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "$script_dir/../parse_result.sh"
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT
checks=0

write_simple_log() {
    local name="$1" body="$2"
    {
        echo "Guest startup message"
        echo FIO_RESULT_BEGIN
        printf '%s\n' "$body"
        echo FIO_RESULT_END
        echo "Guest shutdown message"
    } >"$work_dir/$name.log"
}

write_erofs_log() {
    local name="$1" workload="$2" io_bytes="$3"
    {
        echo FIO_RESULT_BEGIN
        jq -n --arg workload "$workload" --argjson io_bytes "$io_bytes" '{
            "fio version": "fio-3.41",
            jobs: [{
                error: 0,
                "job options": {
                    filename: "/erofs/silesia.tar",
                    rw: $workload,
                    ioengine: "sync",
                    size: "16777216",
                    io_size: ($io_bytes | tostring),
                    bs: "4096",
                    direct: "0",
                    numjobs: "1",
                    iodepth: "1",
                    loops: "1",
                    time_based: "0",
                    ramp_time: "0",
                    invalidate: "0",
                    fsync_on_close: "0",
                    allow_file_create: "0",
                    randrepeat: "1",
                    randseed: "20261110",
                    random_generator: "tausworthe",
                    norandommap: "0"
                },
                read: {
                    bw_bytes: 100000000,
                    io_bytes: $io_bytes,
                    total_ios: ($io_bytes / 4096),
                    short_ios: 0,
                    drop_ios: 0,
                    runtime: 100
                },
                write: {io_bytes: 0}
            }]
        }'
        echo FIO_RESULT_END
    } >"$work_dir/$name.log"
}

expect_bandwidth() {
    local expected="$1"
    shift
    local actual
    actual=$(extract_fio_result "$@")
    [[ "$actual" == "$expected" ]] || {
        echo "FAIL: expected $expected MB/s, got $actual" >&2
        exit 1
    }
    checks=$((checks + 1))
}

expect_failure() {
    if "$@" >"$work_dir/stdout" 2>"$work_dir/stderr"; then
        echo "FAIL: command unexpectedly succeeded: $*" >&2
        exit 1
    fi
    checks=$((checks + 1))
}

write_simple_log seq_read \
    '{"global options":{"rw":"read"},"jobs":[{"error":0,"read":{"bw_bytes":100000000}}]}'
write_simple_log seq_write \
    '{"jobs":[{"error":0,"job options":{"rw":"write"},"write":{"bw_bytes":125000000}}]}'
write_simple_log randread \
    '{"global options":{"rw":"read"},"jobs":[{"error":0,"read":{"bw_bytes":100000000},"job options":{"rw":"randread"}}]}'
write_simple_log fio_error \
    '{"global options":{"rw":"read"},"jobs":[{"error":5,"read":{"bw_bytes":100000000}}]}'
write_simple_log multiple_jobs \
    '{"global options":{"rw":"read"},"jobs":[{"error":0,"read":{"bw_bytes":100000000}},{"error":0,"read":{"bw_bytes":100000000}}]}'
printf 'FIO_RESULT_BEGIN\n{"global options":{"rw":"read"},"jobs":[{"error":0,"read":{"bw_bytes":100000000}}]}\n' \
    >"$work_dir/incomplete.log"
cat "$work_dir/seq_read.log" "$work_dir/seq_read.log" >"$work_dir/duplicate.log"
printf 'FIO_RESULT_BEGIN\n{"jobs": [\nFIO_RESULT_END\n' >"$work_dir/malformed.log"
write_erofs_log erofs_seq_read read 16777216
write_erofs_log erofs_randread_full randread 16777216
write_erofs_log erofs_randread_10m randread 10485760

expect_bandwidth 100 "$work_dir/seq_read.log" read
expect_bandwidth 125 "$work_dir/seq_write.log" write
expect_bandwidth 100 "$work_dir/randread.log" read randread
expect_failure extract_fio_result "$work_dir/seq_read.log" read randread
expect_failure extract_fio_result "$work_dir/randread.log" read
expect_failure extract_fio_result "$work_dir/seq_read.log" write read
# The random sample overrides global rw=read with job rw=randread.
for fixture in fio_error multiple_jobs incomplete duplicate malformed; do
    expect_failure extract_fio_result "$work_dir/$fixture.log" read
done

for bandwidth in 0 -1 null '"100000000"'; do
    jq -n --argjson bandwidth "$bandwidth" \
        '{jobs:[{error:0,"job options":{rw:"read"},read:{bw_bytes:$bandwidth}}]}' >"$work_dir/body.json"
    { echo FIO_RESULT_BEGIN; cat "$work_dir/body.json"; echo FIO_RESULT_END; } >"$work_dir/invalid.log"
    expect_failure extract_fio_result "$work_dir/invalid.log" read
done

for workload in read randread; do
    fixture=erofs_seq_read
    [[ "$workload" != randread ]] || fixture=erofs_randread_full
    extract_fio_json "$work_dir/$fixture.log" >"$work_dir/full.json"
    validate_erofs_fio_result "$work_dir/full.json" "$workload" 16777216 16777216
    checks=$((checks + 1))
done

extract_fio_json "$work_dir/erofs_randread_10m.log" >"$work_dir/valid.json"
validate_erofs_fio_result "$work_dir/valid.json" randread 16777216 10485760
checks=$((checks + 1))
for change in \
    '.jobs[0].read.io_bytes = 10481664' \
    '.jobs[0].read.total_ios = 2559' \
    '.jobs[0].read.short_ios = 1' \
    '.jobs[0].read.drop_ios = 1' \
    '.jobs[0].read.runtime = 0' \
    '.jobs[0].write.io_bytes = 4096' \
    '.jobs[0]."job options".bs = "8192"' \
    '.jobs[0]."job options".size = "10485760"' \
    '.jobs[0]."job options".io_size = "16777216"' \
    '.jobs[0]."job options".rw = "read"' \
    '.jobs[0]."job options".invalidate = "1"' \
    '.jobs[0]."job options".randseed = "1"'; do
    jq "$change" "$work_dir/valid.json" >"$work_dir/invalid.json"
    expect_failure validate_erofs_fio_result "$work_dir/invalid.json" randread 16777216 10485760
done

echo "PASS: $checks fio parser checks"
