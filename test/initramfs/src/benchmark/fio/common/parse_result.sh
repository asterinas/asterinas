#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# Extract one successful, single-job FIO result as decimal MB/s.
extract_fio_result() (
    set -o pipefail
    local output="$1" direction="$2"

    case "$direction" in
        read|write) ;;
        *)
            echo "Error: Invalid FIO result direction '$direction'" >&2
            return 1
            ;;
    esac

    # Reject incomplete or duplicate results instead of parsing a partial run.
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
    ' "$output" | jq -es --arg direction "$direction" '
        if length != 1 then
            error("expected one FIO JSON object")
        else
            .[0]
        end
        | ."global options".rw as $global_direction
        | if (.jobs | type) != "array" or (.jobs | length) != 1 then
            error("expected exactly one FIO job")
        else
            .jobs[0]
        end
        | if .error != 0 then
            error("FIO job failed")
        elif (."job options".rw // $global_direction) != $direction then
            error("FIO job direction does not match the result configuration")
        else
            .[$direction].bw_bytes
        end
        | if type != "number" then
            error("missing numeric FIO bw_bytes")
        elif . <= 0 then
            error("FIO bandwidth must be positive")
        else
            . / 1000000
        end
    '
)
