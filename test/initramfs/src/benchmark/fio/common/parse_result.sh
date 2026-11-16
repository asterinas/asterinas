#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# Extract one complete, successful single-job JSON result from the guest console.
extract_fio_json() (
    set -o pipefail
    awk '
        { sub(/\r$/, "") }
        $0 == "FIO_RESULT_BEGIN" {
            if (started || finished) { invalid = 1; exit 1 }
            started = 1
            next
        }
        $0 == "FIO_RESULT_END" {
            if (!started || finished) { invalid = 1; exit 1 }
            finished = 1
            next
        }
        started && !finished { print }
        END {
            if (invalid || !started || !finished) {
                print "Error: Expected one complete FIO JSON result." > "/dev/stderr"
                exit 1
            }
        }
    ' "$1" | jq -es '
        if length != 1 or (.[0] | type) != "object" then
            error("expected one FIO JSON object")
        else .[0] end
        | if (.jobs | type) != "array" or (.jobs | length) != 1 then
            error("expected exactly one FIO job")
        elif .jobs[0].error != 0 then
            error("FIO job failed")
        else . end
    '
)

# Keep the old two-argument interface; an optional workload enables random I/O.
extract_fio_result() (
    set -o pipefail
    local output="$1" direction="$2" workload="${3:-$2}"
    case "$direction:$workload" in
        read:read|read:randread|write:write|write:randwrite) ;;
        *) echo "Error: Invalid FIO direction/workload '$direction:$workload'" >&2; return 1 ;;
    esac

    extract_fio_json "$output" | jq -e --arg direction "$direction" --arg workload "$workload" '
        ."global options".rw as $global_workload
        | .jobs[0]
        | if (."job options".rw // $global_workload) != $workload then
            error("FIO workload does not match the result configuration")
        else .[$direction].bw_bytes end
        | if type != "number" then
            error("missing numeric FIO bw_bytes")
        elif isfinite | not then
            error("FIO bandwidth must be finite")
        elif . <= 0 then
            error("FIO bandwidth must be positive")
        else . / 1000000 end
    '
)

# Validate the finite, 4 KiB buffered read workload before publishing its bandwidth.
validate_erofs_fio_result() {
    local fio_json="$1" workload="$2" file_bytes="$3" io_bytes="$4"
    jq -e --arg workload "$workload" --argjson file_bytes "$file_bytes" --argjson io_bytes "$io_bytes" '
        def number: tonumber;
        ((."global options" // {}) + (.jobs[0]."job options" // {})) as $options
        | if (.jobs | length) != 1 or .jobs[0].error != 0 then
            error("expected one successful EROFS FIO job")
        elif ($workload != "read" and $workload != "randread")
            or $file_bytes < 10485760 or $file_bytes % 4096 != 0
            or $io_bytes <= 0 or $io_bytes > $file_bytes or $io_bytes % 4096 != 0 then
            error("invalid EROFS workload configuration")
        elif $options.rw != $workload or $options.filename != "/erofs/silesia.tar"
            or $options.ioengine != "sync"
            or ($options.size | number) != $file_bytes
            or ($options.io_size | number) != $io_bytes
            or ($options.bs | number) != 4096
            or ($options.direct | number) != 0
            or ($options.numjobs | number) != 1
            or ($options.iodepth | number) != 1
            or ($options.loops | number) != 1
            or ($options.time_based | number) != 0
            or ($options.ramp_time | number) != 0
            or ($options.invalidate | number) != 0
            or ($options.fsync_on_close | number) != 0
            or ($options.allow_file_create | number) != 0 then
            error("EROFS FIO options do not match the configured workload")
        elif $workload == "randread" and (
            ($options.randrepeat | number) != 1
            or ($options.randseed | number) != 20261110
            or $options.random_generator != "tausworthe"
            or ($options.norandommap | number) != 0) then
            error("EROFS random read options do not match the configured workload")
        elif .jobs[0].read.io_bytes != $io_bytes
            or .jobs[0].read.total_ios != ($io_bytes / 4096)
            or .jobs[0].read.short_ios != 0 or .jobs[0].read.drop_ios != 0
            or .jobs[0].read.runtime <= 0
            or .jobs[0].write.io_bytes != 0 then
            error("EROFS FIO read did not complete the expected I/O")
        else true end
    ' "$fio_json" >/dev/null
}
