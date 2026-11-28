#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

usage() {
    cat <<'HELP'
Usage: test/linux_regression/run.sh [OPTIONS]

Run Asterinas's regression test suite against the pinned prebuilt x86_64
Linux 6.16.0 kernel inside a single QEMU/KVM VM.

Options:
  --timeout DURATION  Limit VM runtime, excluding the build (default: 15m)
  --log-dir DIR       Copy userspace.log and kernel.log to DIR on exit
  -h, --help          Show this help message and exit

Example:
  test/linux_regression/run.sh --timeout 15m --log-dir /tmp/linux-regression
HELP
}

timeout_duration=15m
log_dir=
while (( $# > 0 )); do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --timeout)
            if (( $# < 2 )); then
                echo "error: --timeout requires a duration" >&2
                exit 2
            fi
            timeout_duration=$2
            shift 2
            ;;
        --log-dir)
            if (( $# < 2 )); then
                echo "error: --log-dir requires a directory" >&2
                exit 2
            fi
            log_dir=$2
            shift 2
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [[ ! -r /dev/kvm || ! -w /dev/kvm ]]; then
    echo "error: /dev/kvm is unavailable or lacks read/write permissions" >&2
    exit 1
fi

# Clear `LD_LIBRARY_PATH` in this process so the dev Docker image's custom QEMU
# libraries do not conflict with Nix-built host tools.
unset LD_LIBRARY_PATH

if [[ -n "$log_dir" ]]; then
    mkdir -p "$log_dir"
    log_dir=$(cd "$log_dir" && pwd)
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
initramfs_dir=$(cd "$script_dir/../initramfs" && pwd)

work_dir=$(mktemp -d)
kernel_log="$work_dir/kernel.log"
userspace_log="$work_dir/userspace.log"
# Pre-create both logs so `cleanup` can copy them even if QEMU never starts.
: > "$kernel_log"
: > "$userspace_log"

# Save the requested logs before removing the temporary disk images.
cleanup() {
    local exit_status=$?
    local copy_status=0
    if [[ -n "$log_dir" ]]; then
        cp --remove-destination "$kernel_log" "$log_dir/kernel.log" || copy_status=$?
        cp --remove-destination "$userspace_log" "$log_dir/userspace.log" || copy_status=$?
    fi
    rm -rf "$work_dir"
    if (( exit_status == 0 && copy_status != 0 )); then
        exit "$copy_status"
    fi
}
trap cleanup EXIT

# Build the VM bundle: kernel image, ext4 root disk image, and host tools.
nix-build --tarball-ttl "$((30 * 24 * 3600))" "$script_dir/default.nix" -o "$work_dir/bundle"

# Build temporary data disk images in `$work_dir` using only the bundled tools.
# Isolating `PATH` skips `Makefile`'s host Nix probe, while `SHELL=/bin/sh` keeps host `sh`.
host_bin="$work_dir/bundle/tools/bin"
PATH="$host_bin" "$host_bin/make" SHELL=/bin/sh \
    -C "$initramfs_dir" BUILD_DIR="$work_dir" TARGET_ARCH=x86_64 data-disk-images

# Configure the Linux VM for the regression suite.
qemu_args=(
    -machine 'q35,accel=kvm' -cpu host -m 2G -smp 2 -no-reboot
    # Keep the default VGA device at `00:01.0` for `fs/sysfs/device/pci` without a window.
    -display none -monitor none

    # Route `ttyS0` to `kernel.log` and `ttyS1` to stdout plus `userspace.log`.
    -chardev "file,id=kernel,path=$kernel_log" -serial chardev:kernel
    -chardev "stdio,id=userspace,signal=off,logfile=$userspace_log" -serial chardev:userspace

    -kernel "$work_dir/bundle/kernel"
    # Pass `lsm=capability` so `security/lsm/module_selection` does not expect Yama.
    -append 'root=/dev/vdd rootfstype=ext4 rw rootwait init=/init console=ttyS0,115200 panic=-1 lsm=capability'

    # Pin PCI slots to match `tools/qemu_args.sh` and probe virtio disks as `/dev/vda`..`/dev/vdd`.
    -drive "if=none,id=ext2,file=$work_dir/ext2.img,format=raw"
    -device 'virtio-blk-pci,addr=0x6,drive=ext2'
    -drive "if=none,id=exfat,file=$work_dir/exfat.img,format=raw"
    -device 'virtio-blk-pci,addr=0x7,drive=exfat'
    -drive "if=none,id=ltp,file=$work_dir/ltp_dev.img,format=raw"
    -device 'virtio-blk-pci,addr=0x8,drive=ltp'
    -drive "if=none,id=root,file=$work_dir/bundle/rootfs,format=raw,snapshot=on"
    -device 'virtio-blk-pci,addr=0xc,drive=root'

    -drive "if=none,id=nvme,file=$work_dir/nvme0n1.img,format=raw"
    -device 'nvme,addr=0x4,drive=nvme,serial=nvme0n1'
    -device 'virtio-rng-pci,addr=0x9'
    -nic 'user,model=virtio-net-pci'
)

# Boot the VM with `TMPDIR=$work_dir` so `snapshot=on` overlays are removed on exit.
vm_status=0
TMPDIR="$work_dir" "$host_bin/timeout" --foreground --kill-after=10s "$timeout_duration" \
    "$host_bin/qemu-system-x86_64" "${qemu_args[@]}" || vm_status=$?

# A clean QEMU exit only means poweroff. Require both guest success markers.
if (( vm_status != 0 )) ||
    ! grep -Fxq 'All regression tests passed.' "$userspace_log" ||
    ! grep -Fxq 'Linux regression exit status: 0' "$userspace_log"; then
    echo "Linux regression failed (QEMU exit status: $vm_status). Kernel log tail:" >&2
    tail -n 50 "$kernel_log" >&2 || true
    exit $(( vm_status != 0 ? vm_status : 1 ))
fi
