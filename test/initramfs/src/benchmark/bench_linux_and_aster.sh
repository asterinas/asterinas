#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Ensure all dependencies are installed
if ! command -v yq >/dev/null 2>&1; then
    echo >&2 "Error: missing required tool: yq"
    exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
    echo >&2 "Error: missing required tool: jq"
    exit 1
fi

# Set up paths
BENCHMARK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
source "${BENCHMARK_ROOT}/common/prepare_host.sh"
RESULT_TEMPLATE="${BENCHMARK_ROOT}/result_template.json"
BENCH_LINUX_VIRTIOFS_WORK_DIR=""
BENCH_LINUX_VIRTIOFSD_PID=""
BENCH_LINUX_VIRTIOFS_SOCKET=""
BENCH_VIRTIOFS_CACHE="auto"
BENCH_FILESYSTEM=""
BENCH_OUTPUT_DIR="${BENCH_OUTPUT_DIR:-}"
EROFS_CONDITIONS_FILE=""
EROFS_STAGE="configuration"
# Keep this tag in sync with tools/qemu_args.sh and the virtiofs benchmark run.sh files.
VIRTIOFS_TAG="aster-virtiofs"

# Prints column `column` (a number or `NF`) of the nth line in `file` matching
# `pattern`. The pattern goes through the environment because `awk -v`
# escape-processes its value (gawk turns `\+` into `+`).
extract_result() {
    local file="$1" pattern="$2" column="$3" nth="$4"
    PATTERN="$pattern" awk -v column="$column" \
        '$0 ~ ENVIRON["PATTERN"] { if (column == "NF") print $NF; else print $column }' \
        "$file" | tr -d '\r' | sed 's/[^0-9.]*//g' | sed -n "${nth}p"
}

# Parse benchmark results
parse_raw_results() {
    local search_pattern="$1"
    local nth_occurrence="$2"
    local result_index="$3"
    local result_file="$4"
    local parser="$5"
    local direction="$6"
    local workload="$7" io_bytes="$8"

    # Extract and sanitize numeric results
    local linux_result aster_result
    case "$parser" in
        text)
            linux_result=$(extract_result "$LINUX_OUTPUT" "$search_pattern" "$result_index" "$nth_occurrence")
            aster_result=$(extract_result "$ASTER_OUTPUT" "$search_pattern" "$result_index" "$nth_occurrence")
            ;;
        fio_json)
            source "${BENCHMARK_ROOT}/fio/common/parse_result.sh"
            linux_result=$(extract_fio_result "$LINUX_OUTPUT" "$direction" "$workload")
            aster_result=$(extract_fio_result "$ASTER_OUTPUT" "$direction" "$workload")
            if [[ "$BENCH_FILESYSTEM" == erofs ]]; then
                local system output
                [[ "$io_bytes" != full ]] || io_bytes=$EROFS_FIO_FILE_BYTES
                for system in linux aster; do
                    output=$LINUX_OUTPUT
                    [[ "$system" != aster ]] || output=$ASTER_OUTPUT
                    [[ $(tr -d '\r' <"$output" | grep -c '^BENCHMARK_EXIT_STATUS=') == 1 ]]
                    tr -d '\r' <"$output" | grep -qx 'BENCHMARK_EXIT_STATUS=0'
                    extract_fio_json "$output" > "$BENCH_OUTPUT_DIR/${system}_fio.json"
                    validate_erofs_fio_result "$BENCH_OUTPUT_DIR/${system}_fio.json" \
                        "$workload" "$EROFS_FIO_FILE_BYTES" "$io_bytes"
                done
            fi
            ;;
        *)
            echo "Error: Unknown result parser '$parser'" >&2
            return 1
            ;;
    esac

    # Ensure both results are valid
    if [ -z "${linux_result}" ] || [ -z "${aster_result}" ]; then
        echo "Error: Failed to parse the results from the benchmark output" >&2
        exit 1
    fi

    # Write the results into the template
    yq --arg linux_result "${linux_result}" --arg aster_result "${aster_result}" \
        '(.[] | select(.extra == "linux_result") | .value) |= $linux_result |
         (.[] | select(.extra == "aster_result") | .value) |= $aster_result' \
        "${RESULT_TEMPLATE}" > "${result_file}"
    echo "Results written to ${result_file}"
}

# Generate a new result template based on unit and legend
generate_template() {
    local unit="$1"
    local legend="$2"

    # Replace placeholders with actual system names
    local linux_legend=${legend//"{system}"/"Linux"}
    local asterinas_legend=${legend//"{system}"/"Asterinas"}

    # Generate the result template JSON
    yq -n --arg linux "$linux_legend" --arg aster "$asterinas_legend" --arg unit "$unit" '[
        { "name": $linux, "unit": $unit, "value": 0, "extra": "linux_result" },
        { "name": $aster, "unit": $unit, "value": 0, "extra": "aster_result" }
    ]' > "${RESULT_TEMPLATE}"
}

# Extract the result file path based on benchmark location
extract_result_file() {
    local bench_result="$1"
    local relative_path="${bench_result#*/benchmark/}"
    local first_dir="${relative_path%%/*}"
    local filename=$(basename "$bench_result")

    # Handle different naming conventions for result files
    if [[ "$filename" == bench_* ]]; then
        local second_part=$(dirname "$bench_result" | awk -F"/benchmark/$first_dir/" '{print $2}' | cut -d'/' -f1)
        echo "result_${first_dir}-${second_part}.json"
    else
        local result_file="result_${relative_path//\//-}"
        echo "${result_file/.yaml/.json}"
    fi
}

# Start virtiofsd for the Linux VM and wait for its socket to appear.
start_linux_virtiofsd() {
    "${BENCHMARK_ROOT}/../../../../tools/run_virtiofsd.sh" \
        --cache-mode "$BENCH_VIRTIOFS_CACHE" \
        --work-dir "$BENCH_LINUX_VIRTIOFS_WORK_DIR" \
        &
    BENCH_LINUX_VIRTIOFSD_PID=$!
    local socket_wait_tries=100 # 10 s at 0.1 s per try
    for _ in $(seq 1 "$socket_wait_tries"); do
        [[ -S "$BENCH_LINUX_VIRTIOFS_SOCKET" ]] && break
        sleep 0.1
    done
    if [[ ! -S "$BENCH_LINUX_VIRTIOFS_SOCKET" ]]; then
        echo "Error: virtiofsd did not create its socket within 10 s" >&2
        exit 1
    fi
}

# Stop the Linux-side virtiofsd and remove its work directory.
cleanup_linux_virtiofs() {
    if [[ -n "$BENCH_LINUX_VIRTIOFSD_PID" ]]; then
        kill "$BENCH_LINUX_VIRTIOFSD_PID" 2>/dev/null || true
        wait "$BENCH_LINUX_VIRTIOFSD_PID" 2>/dev/null || true
    fi
    rm -rf "$BENCH_LINUX_VIRTIOFS_WORK_DIR"
}

# Keep the original failure code even if writing the final record also fails.
finish_erofs_conditions() {
    local exit_status=$?
    trap - EXIT
    set +e
    {
        printf '\n## 执行状态与实际命令\n\n'
        printf '整体退出码：`%s`；最后阶段：`%s`。\n\n' "$exit_status" "$EROFS_STAGE"
        local system output
        for system in aster linux; do
            output="$BENCH_OUTPUT_DIR/${system}_output.txt"
            printf '### %s\n\n' "$system"
            printf '[原始日志](%s_output.txt) · [fio JSON](%s_fio.json)\n\n' "$system" "$system"
            if [[ "$system" == aster && -f "$BENCH_OUTPUT_DIR/aster_bundle.toml" ]]; then
                printf '[实际 OSDK 启动配置](aster_bundle.toml)\n\n'
            fi
            if [[ ! -f "$output" ]]; then
                printf '未启动，没有实际执行命令。\n\n'
                continue
            fi
            printf '```text\n'
            # The first line is the exact host argv; the remaining lines come from the guest.
            head -1 "$output"
            tr -d '\r' < "$output" | awk '/^(GUEST_UNAME:|EROFS_PROFILE=|HOST_EXIT_STATUS=|BENCHMARK_EXIT_STATUS=|ASTER_KERNEL_|PREPARED_INITRAMFS_|ASTER_INITRAMFS_|LINUX_INITRAMFS_|INITRAMFS_)/'
            printf '```\n\n'
        done
        if [[ -f "$BENCH_OUTPUT_DIR/aster_fio.json" && -f "$BENCH_OUTPUT_DIR/linux_fio.json" ]]; then
            printf '### 实际 fio 参数\n\n| 参数 | Asterinas | Linux |\n| --- | --- | --- |\n'
            jq -nr --slurpfile aster "$BENCH_OUTPUT_DIR/aster_fio.json" \
                --slurpfile linux "$BENCH_OUTPUT_DIR/linux_fio.json" '
                (($aster[0]."global options" // {}) + $aster[0].jobs[0]."job options") as $a
                | (($linux[0]."global options" // {}) + $linux[0].jobs[0]."job options") as $l
                | ["rw", "size", "io_size", "bs", "ioengine", "direct", "numjobs", "iodepth",
                   "loops", "time_based", "ramp_time", "invalidate", "fsync_on_close",
                   "allow_file_create", "randrepeat", "randseed", "random_generator", "norandommap"][] as $key
                | "| \($key) | \($a[$key] // "未设置") | \($l[$key] // "未设置") |"
            '
        fi
    } >> "$EROFS_CONDITIONS_FILE"
    exit "$exit_status"
}

# Run the specified benchmark with runtime configurations.
run_benchmark() {
    local benchmark="$1"
    local run_mode="$2"
    local runtime_configs_str="$3" # String with key=value pairs, one per line

    # Default values
    local smp_val=1
    local mem_val="8G"
    local aster_scheme_cmd_part="SCHEME=iommu" # Default scheme
    local virtiofs="off"
    local image_profile=""
    local linux_initramfs="${BENCHMARK_ROOT}/../../build/initramfs.cpio.gz"
    local prepared_initramfs_hash=""

    # Process runtime_configs_str to override defaults and gather extra args
    while IFS='=' read -r key value; do
         if [[ -z "$key" ]]; then continue; fi # Skip empty lines/keys
         case "$key" in
             "smp")
                 smp_val="$value"
                 ;;
             "mem")
                 mem_val="$value"
                 ;;
             "aster_scheme")
                 if [[ "$value" == "null" ]]; then
                     aster_scheme_cmd_part="" # Remove default SCHEME=iommu
                 else
                     aster_scheme_cmd_part="SCHEME=${value}" # Override default
                 fi
                 ;;
             "filesystem")
                 BENCH_FILESYSTEM="$value"
                 ;;
             "image_profile")
                 image_profile="$value"
                 ;;
             "virtiofs")
                 virtiofs="$value"
                 ;;
             *)
                 echo "Warning: Unknown runtime configuration key '$key'" >&2
                 exit 1
                 ;;
         esac
     done <<< "$runtime_configs_str"

    if [[ "$BENCH_FILESYSTEM" == erofs ]]; then
        if [[ -z "$BENCH_OUTPUT_DIR" ]]; then
            BENCH_OUTPUT_DIR=$(mktemp -d "${TMPDIR:-/tmp}/erofs-fio-${benchmark##*/}.XXXXXX")
        else
            mkdir -p "$BENCH_OUTPUT_DIR"
            [[ -z $(ls -A "$BENCH_OUTPUT_DIR") ]] || {
                echo "Error: BENCH_OUTPUT_DIR must be empty to preserve previous results" >&2
                return 1
            }
            BENCH_OUTPUT_DIR=$(cd "$BENCH_OUTPUT_DIR" && pwd)
        fi
        LINUX_OUTPUT="$BENCH_OUTPUT_DIR/linux_output.txt"
        ASTER_OUTPUT="$BENCH_OUTPUT_DIR/aster_output.txt"
        RESULT_TEMPLATE="$BENCH_OUTPUT_DIR/result_template.json"
        EROFS_CONDITIONS_FILE="$BENCH_OUTPUT_DIR/conditions.md"
        trap finish_erofs_conditions EXIT
        cat > "$EROFS_CONDITIONS_FILE" <<EOF
# EROFS fio 测试条件

| 条件 | 本次配置 |
| --- | --- |
| 用例 | $benchmark |
| EROFS 镜像配置 | $image_profile |
| vCPU / 内存 | $smp_val / $mem_val |
| 启动顺序 | ${BENCH_SYSTEM_ORDER:-asterinas,linux} |
| 次数 | 每个系统一个新 VM、一个测量 job；本目录保存一次配对尝试 |
| 缓存 | 每次启动新 VM；挂载后只检查路径和大小，不预读文件数据；不清空宿主机/设备缓存 |
| QEMU 数据盘 | format=raw, readonly=on, cache=none, aio=threads |
| 超时 | 每个系统 ${BENCH_TIMEOUT:-900} 秒；包含该命令的构建检查、启动和 fio；前置宿主准备不计入 |
| 负载参数 | 成功后从实际 fio JSON 生成下方参数表；实际 argv、版本和状态保存在原日志 |

开始时间：$(date -u '+%Y-%m-%dT%H:%M:%SZ')。
EOF
        echo "Using EROFS fio work directory $BENCH_OUTPUT_DIR"
        case "$image_profile" in
            plain|lz4hc_c4096) ;;
            *) echo "Error: Unsupported EROFS fio image profile: $image_profile" >&2; return 1 ;;
        esac
        if [[ "$benchmark" != *"_$image_profile" || "$smp_val" != 1 || "$mem_val" != 8G
            || -n "$aster_scheme_cmd_part" || -n "$platform" || "$virtiofs" != off
            || "$run_mode" != guest_only || "${TARGET_ARCH:-x86_64}" != x86_64
            || "${INITRAMFS:-on}" != on || "${INITRAMFS_SKIP_GZIP:-0}" == 1 ]]; then
            echo "Error: EROFS fio requires a matching profile, x86_64, 1 CPU, 8G, gzip initramfs, and the default scheme" >&2
            return 1
        fi
        export EROFS_FIO_PROFILE="$image_profile"
        case "${BENCH_SYSTEM_ORDER:-asterinas,linux}" in
            asterinas,linux|linux,asterinas) ;;
            *) echo "Error: BENCH_SYSTEM_ORDER must be asterinas,linux or linux,asterinas" >&2; return 1 ;;
        esac
        EROFS_STAGE="host preparation"
        echo "Preparing libraries..."
        prepare_libs 2> >(tee -a "$BENCH_OUTPUT_DIR/prepare_output.txt" >&2)
        prepare_fs "$benchmark"
        # Pin Linux to the prepared immutable image even if Asterinas rebuilds the symlink.
        linux_initramfs=$(readlink -f "$linux_initramfs")
        prepared_initramfs_hash=$(sha256sum "$linux_initramfs" | cut -d' ' -f1)
        {
            date -u '+started=%Y-%m-%dT%H:%M:%SZ'
            uname -a
            git -C "${BENCHMARK_ROOT}/../../../.." rev-parse HEAD
            qemu-system-x86_64 --version
            sha256sum "$LINUX_KERNEL" "${BENCHMARK_ROOT}/../../build/initramfs.cpio.gz"
            printf 'system_order=%s\n' "${BENCH_SYSTEM_ORDER:-asterinas,linux}"
            printf 'PREPARED_INITRAMFS_SHA256=%s\n' "$prepared_initramfs_hash"
            printf 'asterinas_cpu=Icelake-Server,+x2apic,+vmx\nlinux_cpu=Icelake-Server,-pcid,+x2apic\n'
            "${BENCHMARK_ROOT}/../../build/initramfs/benchmark/bin/fio" --version
            printf 'FIO_BINARY_SHA256=%s\n' "$(sha256sum "${BENCHMARK_ROOT}/../../build/initramfs/benchmark/bin/fio" | cut -d' ' -f1)"
        } > "$BENCH_OUTPUT_DIR/environment.txt"
        {
            printf '\n## 输入与程序身份\n\n'
            printf '数据文件：`%s/root/silesia.tar`，%s 字节。\n\n' "$EROFS_FIO_WORK_DIR" "$EROFS_FIO_FILE_BYTES"
            printf '镜像：`%s`，%s 字节。\n\n' "$EROFS_FIO_IMAGE" "$(stat -c %s "$EROFS_FIO_IMAGE")"
            printf '[数据集记录](dataset.txt) · [制镜记录](image.txt) · [准备日志](prepare_output.txt)\n\n```text\n'
            sha256sum "$EROFS_FIO_WORK_DIR/root/silesia.tar" "$EROFS_FIO_IMAGE"
            sed -n '/^source_url=/p; /^archive_sha256=/p; /^tar_options=/p' "$BENCH_OUTPUT_DIR/dataset.txt"
            head -6 "$BENCH_OUTPUT_DIR/image.txt"
            cat "$BENCH_OUTPUT_DIR/environment.txt"
            printf '```\n'
        } >> "$EROFS_CONDITIONS_FILE"
    elif [[ -n "$BENCH_FILESYSTEM" || -n "$image_profile" ]]; then
        echo "Error: Unsupported filesystem/image profile configuration" >&2
        return 1
    else
        echo "Preparing libraries..."
        prepare_libs
    fi

    # Prepare commands for Asterinas and Linux using arrays
    local asterinas_cmd_arr=(make run_kernel "BENCHMARK=${benchmark}")
    # Add scheme part only if it's not empty and the platform is not TDX (OSDK doesn't support multiple SCHEME)
    [[ -n "$aster_scheme_cmd_part" && "$platform" != "tdx" ]] && asterinas_cmd_arr+=("$aster_scheme_cmd_part")
    asterinas_cmd_arr+=(
        "SMP=${smp_val}"
        "MEM=${mem_val}"
        ENABLE_KVM=1
        RELEASE_LTO=1
        NETDEV=tap
        VHOST=on
    )
    if [[ "$platform" == "tdx" ]]; then
        asterinas_cmd_arr+=(INTEL_TDX=1)
    fi
    if [[ "$BENCH_FILESYSTEM" == erofs ]]; then
        # Make's default scheme is the literal empty quoted string.
        asterinas_cmd_arr+=('SCHEME=""' INTEL_TDX=0 TARGET_ARCH=x86_64 INITRAMFS=on)
    fi

    if [[ "$virtiofs" == "on" ]]; then
        case "$BENCH_VIRTIOFS_CACHE" in
            auto|always|never|metadata) ;;
            *)
                echo "Error: invalid VIRTIOFS_CACHE value '$BENCH_VIRTIOFS_CACHE'" >&2
                exit 1
                ;;
        esac
        BENCH_LINUX_VIRTIOFS_WORK_DIR=$(mktemp -d -p /tmp asterinas-bench-linux-virtiofs-XXXXXX)
        # run_virtiofsd.sh uses vfs.sock under its work directory.
        BENCH_LINUX_VIRTIOFS_SOCKET="${BENCH_LINUX_VIRTIOFS_WORK_DIR}/vfs.sock"
        trap cleanup_linux_virtiofs EXIT
        asterinas_cmd_arr+=(
            VIRTIOFS=on
            "VIRTIOFS_CACHE=${BENCH_VIRTIOFS_CACHE}"
        )
    fi

    local linux_cmd_arr=(
        qemu-system-x86_64
        --no-reboot
        -smp "${smp_val}"
        -m "${mem_val}"
        -machine q35,kernel-irqchip=split
        --enable-kvm
        -kernel "${LINUX_KERNEL}"
        -initrd "$linux_initramfs"
        -drive "if=none,format=raw,id=x0,file=${BENCHMARK_ROOT}/../../build/ext2.img"
        -device "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off"
        -drive "if=none,format=raw,id=nvme0n1,file=${BENCHMARK_ROOT}/../../build/nvme0n1.img"
        -device "nvme,drive=nvme0n1,serial=nvme0n1"
        -device "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off,csum=off,ctrl_guest_offloads=off,ctrl_mac_addr=off,ctrl_rx_extra=off,ctrl_rx=off,ctrl_vlan=off,ctrl_vq=off,event_idx=off,guest_announce=off,guest_csum=off,guest_ecn=off,guest_tso4=off,guest_tso6=off,guest_ufo=off,guest_uso4=off,guest_uso6=off,host_ecn=off,host_tso4=off,host_tso6=off,host_ufo=off,host_uso=off,indirect_desc=off,mrg_rxbuf=off,queue_reset=off"
        -append "console=ttyS0 rdinit=/benchmark/common/bench_runner.sh ${benchmark} linux mitigations=off hugepages=0 transparent_hugepage=never quiet"
        -netdev "tap,id=net01,script=${BENCHMARK_ROOT}/../../../../tools/net/qemu-ifup.sh,downscript=${BENCHMARK_ROOT}/../../../../tools/net/qemu-ifdown.sh,vhost=on"
        -nographic
    )
    if [[ "$BENCH_FILESYSTEM" == erofs ]]; then
        linux_cmd_arr+=(
            -drive "if=none,format=raw,id=erofsfio,file=${EROFS_FIO_IMAGE},readonly=on,cache=none,aio=threads"
            -device "virtio-blk-pci,bus=pcie.0,addr=0xa,drive=erofsfio,serial=erofs-fio,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off"
        )
        # Linux sees one default virtio disk; Asterinas sees three.
        local index
        for index in "${!linux_cmd_arr[@]}"; do
            if [[ "${linux_cmd_arr[$index]}" == console=* ]]; then
                linux_cmd_arr[$index]="console=ttyS0 mitigations=off hugepages=0 transparent_hugepage=never quiet rdinit=/benchmark/common/bench_runner.sh -- ${benchmark} linux /dev/vdb ${EROFS_FIO_FILE_BYTES} ${EROFS_FIO_PROFILE}"
            fi
        done
    fi
    if [[ "$platform" != "tdx" ]]; then
        linux_cmd_arr+=(
            -cpu Icelake-Server,-pcid,+x2apic
        )
    else
        linux_cmd_arr+=(
            -machine confidential-guest-support=tdx0
            -cpu host,-kvm-steal-time,pmu=off
            -bios "${OVMF_DIR:-/root/ovmf/release}/OVMF.fd"
            -nodefaults
            -serial stdio
            -object '{ "qom-type": "tdx-guest", "id": "tdx0", "sept-ve-disable": true, "quote-generation-socket": { "type": "vsock", "cid": "1", "port": "4050" } }'
        )
    fi

    if [[ "$virtiofs" == "on" ]]; then
        # The Linux VM uses the same mount tag as Asterinas.
        linux_cmd_arr+=(
            -object "memory-backend-memfd,id=mem0,size=${mem_val},share=on"
            -numa node,memdev=mem0
            -chardev "socket,id=char0,path=${BENCH_LINUX_VIRTIOFS_SOCKET}"
            -device "vhost-user-fs-pci,chardev=char0,tag=${VIRTIOFS_TAG}"
        )
    fi

    # Run the benchmark depending on the mode
    case "${run_mode}" in
        "guest_only")
            if [[ "$BENCH_FILESYSTEM" == erofs ]]; then
                local system output system_order="${BENCH_SYSTEM_ORDER:-asterinas,linux}"
                local -a command systems=("${system_order%,*}" "${system_order#*,}")
                source "${BENCHMARK_ROOT}/fio/common/parse_result.sh"
                for system in "${systems[@]}"; do
                    if [[ "$system" == asterinas ]]; then
                        command=("${asterinas_cmd_arr[@]}")
                        output=$ASTER_OUTPUT
                    else
                        command=("${linux_cmd_arr[@]}")
                        output=$LINUX_OUTPUT
                    fi
                    echo "Running benchmark ${benchmark} on ${system}..."
                    printf '%q ' "${command[@]}" > "$output"
                    printf '\n' >> "$output"
                    EROFS_STAGE="$system build/boot/fio"
                    local status=0
                    timeout --foreground --kill-after=10 "${BENCH_TIMEOUT:-900}" \
                        "${command[@]}" 2>&1 | tee -a "$output" || status=$?
                    printf '\nHOST_EXIT_STATUS=%s\n' "$status" | tee -a "$output"
                    local actual_initramfs="$linux_initramfs" initramfs_prefix=LINUX
                    if [[ "$system" == asterinas ]]; then
                        actual_initramfs=""
                        initramfs_prefix=ASTER
                        local kernel_bundle="${BENCHMARK_ROOT}/../../../../target/osdk/asterinas/asterinas-osdk-bin"
                        local kernel_hash
                        if tr -d '\r' < "$output" | grep -E '^(GUEST_UNAME:|qemu-system-x86_64:)' >/dev/null; then
                            if tr -d '\r' < "$output" | grep '^GUEST_UNAME:' >/dev/null; then
                                echo 'ASTER_KERNEL_IDENTITY=guest start confirmed' | tee -a "$output"
                            else
                                echo 'ASTER_KERNEL_IDENTITY=QEMU start observed; guest execution unconfirmed' | tee -a "$output"
                            fi
                            local bundle_dir="${kernel_bundle%/*}" bundle_initramfs
                            if [[ -f "$bundle_dir/bundle.toml" ]]; then
                                cp "$bundle_dir/bundle.toml" "$BENCH_OUTPUT_DIR/aster_bundle.toml"
                                bundle_initramfs=$(awk -F '"' '
                                    $0 == "[initramfs]" { in_initramfs = 1; next }
                                    /^\[/ { in_initramfs = 0 }
                                    in_initramfs && /^path = / { print $2; exit }
                                ' "$BENCH_OUTPUT_DIR/aster_bundle.toml")
                                case "$bundle_initramfs" in
                                    /*) actual_initramfs="$bundle_initramfs" ;;
                                    ?*) actual_initramfs="$bundle_dir/$bundle_initramfs" ;;
                                esac
                            fi
                            if kernel_hash=$(sha256sum "$kernel_bundle"); then
                                printf 'ASTER_KERNEL_SHA256=%s\n' "$kernel_hash" | tee -a "$output"
                                printf '%s\n' "$kernel_hash" >> "$BENCH_OUTPUT_DIR/environment.txt"
                            else
                                echo 'ASTER_KERNEL_IDENTITY=unconfirmed: launch observed but bundle is unavailable' | tee -a "$output"
                            fi
                        else
                            echo 'ASTER_KERNEL_IDENTITY=unconfirmed: no guest or QEMU start marker; previous bundle not attributed to this run' | tee -a "$output"
                        fi
                    fi
                    if [[ "$status" == 0 ]] && tr -d '\r' < "$output" | grep -qx 'BENCHMARK_EXIT_STATUS=0'; then
                        local fio_json="$BENCH_OUTPUT_DIR/${system/asterinas/aster}_fio.json"
                        if extract_fio_json "$output" > "$fio_json.tmp"; then
                            mv "$fio_json.tmp" "$fio_json"
                        else
                            rm -f "$fio_json.tmp"
                        fi
                    fi
                    local actual_initramfs_hash
                    printf 'PREPARED_INITRAMFS_SHA256=%s\n' "$prepared_initramfs_hash" | tee -a "$output"
                    if [[ -n "$actual_initramfs" && -f "$actual_initramfs" ]] \
                        && actual_initramfs_hash=$(sha256sum "$actual_initramfs" | cut -d' ' -f1); then
                        printf '%s_INITRAMFS_PATH=%s\n%s_INITRAMFS_SHA256=%s\n' \
                            "$initramfs_prefix" "$actual_initramfs" "$initramfs_prefix" "$actual_initramfs_hash" | tee -a "$output"
                        if [[ "$actual_initramfs_hash" == "$prepared_initramfs_hash" ]]; then
                            echo 'INITRAMFS_MATCH=1' | tee -a "$output"
                        else
                            echo 'INITRAMFS_MATCH=0: actual guest image differs from prepared image' | tee -a "$output"
                            if [[ "$status" == 0 ]]; then
                                status=1
                                EROFS_STAGE="$system initramfs verification"
                            fi
                        fi
                    else
                        echo 'INITRAMFS_IDENTITY=unconfirmed: no launched guest image could be identified' | tee -a "$output"
                        if [[ "$status" == 0 ]]; then
                            status=1
                            EROFS_STAGE="$system initramfs verification"
                        fi
                    fi
                    [[ "$status" == 0 ]] || return "$status"
                done
            else
                echo "Running benchmark ${benchmark} on Asterinas..."
                "${asterinas_cmd_arr[@]}" 2>&1 | tee "${ASTER_OUTPUT}"
                prepare_fs "$benchmark"
                if [[ "$virtiofs" == "on" ]]; then
                    start_linux_virtiofsd
                fi
                echo "Running benchmark ${benchmark} on Linux..."
                "${linux_cmd_arr[@]}" 2>&1 | tee "${LINUX_OUTPUT}"
            fi
            ;;
        "host_guest")
            # Note: host_guest_bench_runner.sh expects commands as single strings.
            # We need to reconstruct the string representation for compatibility.
            # Use printf %q to quote arguments safely.
            local asterinas_cmd_str
            printf -v asterinas_cmd_str '%q ' "${asterinas_cmd_arr[@]}"
            local linux_cmd_str
            printf -v linux_cmd_str '%q ' "${linux_cmd_arr[@]}"

            if [[ "$virtiofs" == "on" ]]; then
                start_linux_virtiofsd
            fi
            echo "Running benchmark ${benchmark} on host and guest..."
            bash "${BENCHMARK_ROOT}/common/host_guest_bench_runner.sh" \
                "${BENCHMARK_ROOT}/${benchmark}" \
                "${asterinas_cmd_str}" \
                "${linux_cmd_str}" \
                "${ASTER_OUTPUT}" \
                "${LINUX_OUTPUT}" \
                "${benchmark}"
            ;;
        *)
            echo "Error: Unknown benchmark type '${run_mode}'" >&2
            exit 1
            ;;
    esac
}

# Parse the benchmark configuration
parse_results() {
    local bench_result="$1"

    local search_pattern nth_occurrence result_index unit legend parser direction workload io_bytes result_file
    search_pattern=$(yq -r '.result_extraction.search_pattern // empty' "$bench_result")
    nth_occurrence=$(yq -r '.result_extraction.nth_occurrence // 1' "$bench_result")
    result_index=$(yq -r '.result_extraction.result_index // empty' "$bench_result")
    unit=$(yq -r '.chart.unit // empty' "$bench_result")
    legend=$(yq -r '.chart.legend // "{system}"' "$bench_result")
    parser=$(yq -r '.result_extraction.parser // "text"' "$bench_result")
    direction=$(yq -r '.result_extraction.direction // empty' "$bench_result")
    workload=$(yq -r '.result_extraction.workload // empty' "$bench_result")
    io_bytes=$(yq -r '.result_extraction.io_bytes // empty' "$bench_result")

    if [[ "$parser" == "fio_json" && "$unit" != "MB/s" ]]; then
        echo "Error: The fio_json parser produces results in MB/s." >&2
        return 1
    fi

    generate_template "$unit" "$legend"
    result_file=$(extract_result_file "$bench_result")
    parse_raw_results "$search_pattern" "$nth_occurrence" "$result_index" "$result_file" "$parser" "$direction" "$workload" "$io_bytes"
}

# Clean up temporary files
cleanup() {
    echo "Cleaning up..."
    if [[ "$BENCH_FILESYSTEM" == erofs ]]; then
        trap - EXIT
        rm -rf "$BENCH_OUTPUT_DIR"
    else
        rm -f "${LINUX_OUTPUT}" "${ASTER_OUTPUT}" "${RESULT_TEMPLATE}"
    fi
}

# Main function to coordinate the benchmark run
main() {
    local benchmark="${1:-}"
    local platform="${2:-}"

    if [[ -z "$benchmark" || ! -d "${BENCHMARK_ROOT}/${benchmark}" ]]; then
        echo "Error: No benchmark specified" >&2
        exit 1
    fi
    echo "Running benchmark $benchmark..."

    # Determine the run mode (host-only or host-guest)
    local run_mode="guest_only"
    [[ -f "${BENCHMARK_ROOT}/${benchmark}/host.sh" ]] && run_mode="host_guest"

    local bench_result="${BENCHMARK_ROOT}/${benchmark}/bench_result.yaml"
    local runtime_configs_str=""

    # Try reading from single result file first
    if [[ -f "$bench_result" ]]; then
        # Read runtime_config object, convert to key=value lines, ensuring value is string
        runtime_configs_str=$(yq -r '(.runtime_config // {}) | to_entries | .[] | .key + "=" + (.value | tostring)' "$bench_result")
    else
        # If not found, try reading from the first file in bench_results/ that has a non-empty runtime_config
        for job_yaml in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            if [[ -f "$job_yaml" ]]; then
                echo "Reading runtime configurations from $job_yaml..."
                # Read runtime_config object, convert to key=value lines, ensuring value is string
                runtime_configs_str=$(yq -r '(.runtime_config // {}) | to_entries | .[] | .key + "=" + (.value | tostring)' "$job_yaml")
                # Check if runtime_config was actually found and non-empty
                if [[ -n "$runtime_configs_str" ]]; then
                    break # Found it, stop looking
                fi
            fi
        done
    fi

    # Run the benchmark, passing the config string
    run_benchmark "$benchmark" "$run_mode" "$runtime_configs_str"

    # Parse results if benchmark configuration exists
    [[ "$BENCH_FILESYSTEM" != erofs ]] || EROFS_STAGE="result validation"
    if [[ -f "$bench_result" ]]; then
        parse_results "$bench_result"
    else
        for job in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            [[ -f "$job" ]] && parse_results "$job"
        done
    fi

    # Cleanup temporary files
    cleanup
    [[ "$BENCH_FILESYSTEM" != erofs ]] || EROFS_STAGE="complete"
    echo "Benchmark completed successfully."
}

main "$@"
