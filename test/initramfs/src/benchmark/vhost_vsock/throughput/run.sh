#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

if [ ! -e /dev/vhost-vsock ]; then
    modprobe vhost_vsock 2>/dev/null || true
fi
if [ ! -e /dev/vhost-vsock ]; then
    echo "Error: /dev/vhost-vsock is unavailable; build vhost_vsock into the kernel or include its modules." >&2
    exit 1
fi

run_vhost_user_client() {
    /benchmark/bin/vhost_vsock_bench \
        --backend vhost-user \
        --vhost-user-socket "$1" \
        --host-socket "$2" \
        --direction "$3" \
        --buf-size "$4" \
        --bytes "$5" \
        --warmup-bytes 0
}

run_vhost_user_once() (
    direction="$1"
    buffer_size="$2"
    total_bytes="$3"
    phase="$4"
    control_socket="/tmp/vhost-user-vsock-${direction}-${buffer_size}-${phase}-$$.sock"
    host_socket="/tmp/vhost-user-vsock-host-${direction}-${buffer_size}-${phase}-$$.sock"

    /benchmark/bin/vhost-device-vsock \
        --guest-cid 42 \
        --socket "${control_socket}" \
        --uds-path "${host_socket}" \
        --tx-buffer-size 262144 \
        --queue-size 256 &
    VHOST_USER_PID=$!
    trap 'kill "${VHOST_USER_PID}" 2>/dev/null || true; wait "${VHOST_USER_PID}" 2>/dev/null || true' EXIT HUP INT TERM

    retries=0
    while [ ! -S "${control_socket}" ] || [ ! -S "${host_socket}" ]; do
        retries=$((retries + 1))
        if [ "${retries}" -ge 300 ]; then
            echo "Error: timed out waiting for vhost-device-vsock sockets." >&2
            exit 1
        fi
        sleep 0.1
    done

    if [ "${phase}" = "warmup" ]; then
        run_vhost_user_client "${control_socket}" "${host_socket}" \
            "${direction}" "${buffer_size}" "${total_bytes}" >/dev/null
    else
        run_vhost_user_client "${control_socket}" "${host_socket}" \
            "${direction}" "${buffer_size}" "${total_bytes}"
    fi

    kill "${VHOST_USER_PID}" 2>/dev/null || true
    wait "${VHOST_USER_PID}" 2>/dev/null || true
    trap - EXIT HUP INT TERM
)

run_vhost_user_benchmark() {
    direction="$1"
    buffer_size="$2"
    total_bytes="$3"
    warmup_bytes="$4"

    run_vhost_user_once "${direction}" "${buffer_size}" \
        "${warmup_bytes}" warmup
    run_vhost_user_once "${direction}" "${buffer_size}" \
        "${total_bytes}" measure
}

echo "*** Running /dev/vhost-vsock and vhost-user-vsock throughput benchmarks ***"

for direction in h2g g2h; do
    for buffer_size in 64 4K 64K; do
        case "${buffer_size}" in
            64)
                total_bytes=64M
                warmup_bytes=1M
                ;;
            4K)
                total_bytes=1G
                warmup_bytes=16M
                ;;
            64K)
                total_bytes=1G
                warmup_bytes=64M
                ;;
        esac
        /benchmark/bin/vhost_vsock_bench \
            --backend vhost \
            --direction "${direction}" \
            --buf-size "${buffer_size}" \
            --bytes "${total_bytes}" \
            --warmup-bytes "${warmup_bytes}"
        run_vhost_user_benchmark "${direction}" "${buffer_size}" \
            "${total_bytes}" "${warmup_bytes}"
    done
done
