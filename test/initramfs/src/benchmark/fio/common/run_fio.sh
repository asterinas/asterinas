#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

if [ -n "${EROFS_FIO_DEVICE:-}" ]; then
    set -u
    workload=${1:?Usage: run_fio.sh <read|randread> <full|10485760>}
    io_bytes=${2:?Missing I/O size}
    device=$EROFS_FIO_DEVICE
    file_bytes=${EROFS_FIO_FILE_BYTES:?Missing EROFS fio file size}
    profile=${EROFS_FIO_PROFILE:?Missing EROFS fio image profile}
    case "$profile" in plain|lz4hc_c4096) ;; *) exit 1 ;; esac
    case "$workload" in read|randread) ;; *) exit 1 ;; esac
    case "$file_bytes" in ''|*[!0-9]*) exit 1 ;; esac
    [ "$file_bytes" -ge 10485760 ] && [ $((file_bytes % 4096)) -eq 0 ]
    case "$io_bytes" in full) io_bytes=$file_bytes ;; 10485760) ;; *) exit 1 ;; esac

    echo "*** Running the FIO $workload test (EROFS, profile=$profile) ***"
    echo "EROFS_PROFILE=$profile device=$device file_bytes=$file_bytes io_bytes=$io_bytes"
    mkdir -p /erofs
    mount -t erofs -o ro "$device" /erofs
    # Always try to unmount, preserving an earlier failure as the final exit status.
    trap 'status=$?; trap - EXIT; umount /erofs || { [ "$status" -ne 0 ] || status=1; }; exit "$status"' EXIT
    [ -f /erofs/silesia.tar ]
    [ "$(stat -c %s /erofs/silesia.tar)" = "$file_bytes" ]

    set -- --readonly --name=erofs-read --filename=/erofs/silesia.tar \
        --rw="$workload" --size="$file_bytes" --io_size="$io_bytes" --bs=4096 \
        --ioengine=sync --direct=0 --numjobs=1 --iodepth=1 --loops=1 \
        --time_based=0 --ramp_time=0 --invalidate=0 --fsync_on_close=0 --allow_file_create=0
    if [ "$workload" = randread ]; then
        set -- "$@" --randrepeat=1 --randseed=20261110 --random_generator=tausworthe --norandommap=0
    fi
fi

# Delimit the JSON so the host can separate it from guest console messages.
# Keep these markers in sync with parse_result.sh.
echo FIO_RESULT_BEGIN
/benchmark/bin/fio "$@" --output-format=json --eta=never
echo FIO_RESULT_END
