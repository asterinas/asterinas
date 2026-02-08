#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# This script is used to generate QEMU arguments for OSDK.
# Usage: `qemu_args.sh [scheme]`
#  - scheme: "normal", "test", "microvm", "iommu", "tdx" or "riscv";
# Other arguments are configured via environmental variables:
#  - OVMF: "on" or "off";
#  - BOOT_METHOD: "qemu-direct", "grub-rescue-iso" or "grub-qcow2";
#  - BOOT_PROTOCOL: "multiboot", "multiboot2", "linux-legacy32", "linux-efi-pe64" or "linux-efi-handover64";
#  - NETDEV: "user" or "tap";
#  - VHOST: "off" or "on";
#  - VSOCK: "off" or "on";
#  - VIRTIOFS: "off" or "on";
#  - VIRTIOFS_TAG: mount tag for virtio-fs device;
#  - VIRTIOFS_SOCKET: vhost-user socket path for the virtio-fs server.
#  - CONSOLE: "hvc0" to enable virtio console;
#  - SMP: number of CPUs;
#  - MEM: amount of memory, e.g. "8G";
#  - VNC_PORT: VNC port, default is "42";
#  - ATTACH_XFSTESTS_IMAGES: "true" or "false", whether to attach xfstests images (xfstests_test.img and xfstests_scratch.img) to the VM. Defaults to auto-detection from ENABLE_CONFORMANCE_TEST + CONFORMANCE_TEST_SUITE.

# ==========================================================
# 1. Environment & Defaults
# ==========================================================

SCHEME=${1:-"normal"}
OVMF=${OVMF:-"on"}
VHOST=${VHOST:-"off"}
VSOCK=${VSOCK:-"off"}
VIRTIOFS=${VIRTIOFS:-"off"}
NETDEV=${NETDEV:-"user"}
CONSOLE=${CONSOLE:-"hvc0"}
SMP=${SMP:-1}
MEM=${MEM:-"8G"}
VNC_PORT=${VNC_PORT:-"42"}

ATTACH_XFSTESTS_IMAGES=${ATTACH_XFSTESTS_IMAGES:-false}
if [ "${ENABLE_CONFORMANCE_TEST:-"false"}" = "true" ] && \
   [ "${CONFORMANCE_TEST_SUITE:-"ltp"}" = "xfstests" ]; then
    ATTACH_XFSTESTS_IMAGES="true"
fi
VIRTIOFS_TAG=${VIRTIOFS_TAG:-"aster-virtiofs"}
VIRTIOFS_SOCKET=${VIRTIOFS_SOCKET:-"/tmp/vhostqemu/vfs.sock"}

SSH_RAND_PORT=${SSH_PORT:-$(shuf -i 1024-65535 -n 1)}
NGINX_RAND_PORT=${NGINX_PORT:-$(shuf -i 1024-65535 -n 1)}
REDIS_RAND_PORT=${REDIS_PORT:-$(shuf -i 1024-65535 -n 1)}
IPERF_RAND_PORT=${IPERF_PORT:-$(shuf -i 1024-65535 -n 1)}
LMBENCH_TCP_LAT_RAND_PORT=${LMBENCH_TCP_LAT_PORT:-$(shuf -i 1024-65535 -n 1)}
LMBENCH_TCP_BW_RAND_PORT=${LMBENCH_TCP_BW_PORT:-$(shuf -i 1024-65535 -n 1)}
MEMCACHED_RAND_PORT=${MEMCACHED_PORT:-$(shuf -i 1024-65535 -n 1)}

qemu_args=()

# ==========================================================
# 2. Backend Configuration
# ==========================================================

# --- Network Backend ---
if [ "$NETDEV" = "user" ]; then
    echo "[$SCHEME] Forwarded QEMU guest port: $SSH_RAND_PORT->22; $NGINX_RAND_PORT->8080 $REDIS_RAND_PORT->6379 $IPERF_RAND_PORT->5201 $LMBENCH_TCP_LAT_RAND_PORT->31234 $LMBENCH_TCP_BW_RAND_PORT->31236 $MEMCACHED_RAND_PORT->11211" 1>&2
    NET_BACKEND_ARGS=(
        "-netdev" "user,id=net01,hostfwd=tcp::$SSH_RAND_PORT-:22,hostfwd=tcp::$NGINX_RAND_PORT-:8080,hostfwd=tcp::$REDIS_RAND_PORT-:6379,hostfwd=tcp::$IPERF_RAND_PORT-:5201,hostfwd=tcp::$LMBENCH_TCP_LAT_RAND_PORT-:31234,hostfwd=tcp::$LMBENCH_TCP_BW_RAND_PORT-:31236,hostfwd=tcp::$MEMCACHED_RAND_PORT-:11211"
    )
    NET_FEATURES=",mrg_rxbuf=off,ctrl_rx=off,ctrl_rx_extra=off,ctrl_vlan=off,ctrl_vq=off,ctrl_guest_offloads=off,ctrl_mac_addr=off,event_idx=off,queue_reset=off,guest_announce=off,indirect_desc=off"
elif [ "$NETDEV" = "tap" ]; then
    THIS_SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    NET_BACKEND_ARGS=(
        "-netdev" "tap,id=net01,script=$THIS_SCRIPT_DIR/net/qemu-ifup.sh,downscript=$THIS_SCRIPT_DIR/net/qemu-ifdown.sh,vhost=$VHOST"
    )
    NET_FEATURES=",csum=off,guest_csum=off,ctrl_guest_offloads=off,guest_tso4=off,guest_tso6=off,guest_ecn=off,guest_ufo=off,host_tso4=off,host_tso6=off,host_ecn=off,host_ufo=off,mrg_rxbuf=off,ctrl_vq=off,ctrl_rx=off,ctrl_vlan=off,ctrl_rx_extra=off,guest_announce=off,ctrl_mac_addr=off,host_ufo=off,guest_uso4=off,guest_uso6=off,host_uso=off"
else
    echo "Invalid NETDEV setting" 1>&2
    NET_BACKEND_ARGS=("-nic" "none")
    NET_FEATURES=""
fi

# Optional QEMU arguments. Opt in them manually if needed.
# QEMU_OPT_ARG_DUMP_PACKETS="-object filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap"
if [ -n "$QEMU_OPT_ARG_DUMP_PACKETS" ]; then
    NET_BACKEND_ARGS+=($QEMU_OPT_ARG_DUMP_PACKETS)
fi

# --- Drive Backend ---
DRIVE_BACKEND_ARGS=(
    "-drive" "if=none,format=raw,id=x0,file=./test/initramfs/build/ext2.img"
    "-drive" "if=none,format=raw,id=x1,file=./test/initramfs/build/exfat.img"
)

if [ "$ATTACH_XFSTESTS_IMAGES" = "true" ]; then
    DRIVE_BACKEND_ARGS+=(
        "-drive" "if=none,format=raw,id=x2,file=./test/initramfs/build/xfstests_test.img"
        "-drive" "if=none,format=raw,id=x3,file=./test/initramfs/build/xfstests_scratch.img"
    )
fi

# --- Console Backend ---
if [ "$CONSOLE" = "hvc0" ]; then
    # Kernel logs are printed to all consoles. Redirect serial output to a file to avoid duplicate logs.
    CONSOLE_ARGS=(
        "-device" "virtconsole,chardev=mux"
        "-serial" "file:qemu-serial.log"
    )
else
    CONSOLE_ARGS=("-serial" "chardev:mux")
fi

if [ "$SCHEME" = "riscv" ]; then
    # NOTE: The `/etc/profile.d/init.sh` assumes that `ext2.img` appears as the first block device (`/dev/vda`).
    # The ordering below ensures `x1` (ext2.img) is discovered before `x0`, maintaining this assumption.
    # TODO: Once UUID-based mounting is implemented, this strict ordering will no longer be required.
    qemu_args=(
        "-cpu" "rv64,svpbmt=true,zkr=true"
        "-machine" "virt"
        "-m" "$MEM"
        "-smp" "$SMP"
        "--no-reboot"
        "-nographic"
        "-display" "none"
        "-monitor" "chardev:mux"
        "-chardev" "stdio,id=mux,mux=on,signal=off,logfile=qemu.log"
        "-drive" "if=none,format=raw,id=x0,file=./test/initramfs/build/ext2.img"
        "-drive" "if=none,format=raw,id=x1,file=./test/initramfs/build/exfat.img"
        "-device" "virtio-blk-device,drive=x1"
        "-device" "virtio-blk-device,drive=x0"
        "-device" "virtio-keyboard-device"
        "-device" "virtio-serial-device"
    )
    qemu_args+=("${CONSOLE_ARGS[@]}")

    echo "${qemu_args[*]}"
    exit 0
fi

# --- Block Device Options ---
BLK_OPTS="disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off"

# ==========================================================
# 3. Mode Selection & Construction
# ==========================================================

VIRTIO_BUS="pci"
CPU_SCHEMEL="Icelake-Server,+x2apic"
EXTRA_DEV_FLAGS=""

case "$SCHEME" in
    "tdx")
        TDX_OBJECT='{ "qom-type": "tdx-guest", "id": "tdx0", "sept-ve-disable": true, "quote-generation-socket": { "type": "vsock", "cid": "1", "port": "4050" } }'

        qemu_args+=(
            "-machine" "q35,kernel-irqchip=split,confidential-guest-support=tdx0"
            "-bios" "/root/ovmf/release/OVMF.fd"
            "-cpu" "host,-kvm-steal-time,pmu=off"
            "-object" "'$TDX_OBJECT'"
            "-vga" "none"
            "-nodefaults"
        )

        OVMF="off_handled"
        ;;

    "microvm")
        VIRTIO_BUS="device"
        OVMF="off"

        qemu_args+=(
            "-machine" "microvm,rtc=on"
            "-nodefaults"
            "-no-user-config"
            "-cpu" "$CPU_SCHEMEL"
        )
        ;;

    "iommu")
        if [ "$OVMF" = "off" ]; then
            echo "Warning: OVMF is off, enabling it for IOMMU support." 1>&2
            OVMF="on"
        fi

        EXTRA_DEV_FLAGS=",iommu_platform=on,ats=on"

        qemu_args+=(
            "-machine" "q35,kernel-irqchip=split"
            "-cpu" "$CPU_SCHEMEL"
            "-device" "intel-iommu,intremap=on,device-iotlb=on"
            "-device" "ioh3420,id=pcie.0,chassis=1"
            "-display" "vnc=0.0.0.0:$VNC_PORT"
        )
        ;;

    *)
        qemu_args+=(
            "-machine" "q35,kernel-irqchip=split"
            "-cpu" "$CPU_SCHEMEL"
            "-display" "vnc=0.0.0.0:$VNC_PORT"
        )
        ;;
esac

# ==========================================================
# 4. Assembling Common Components
# ==========================================================

qemu_args+=(
    "-smp" "$SMP"
    "-m" "$MEM"
    "--no-reboot"
    "-nographic"
    "-monitor" "pty"
    "-monitor" "chardev:mux"
    "-chardev" "stdio,id=mux,mux=on,signal=off,logfile=qemu.log"
    "-device" "isa-debug-exit,iobase=0xf4,iosize=0x04"
)

qemu_args+=("${NET_BACKEND_ARGS[@]}")
qemu_args+=("${DRIVE_BACKEND_ARGS[@]}")

# ==========================================================
# 5. Device Attachment
# ==========================================================

if [ "$VIRTIO_BUS" = "pci" ]; then
    if [ "$SCHEME" = "tdx" ]; then
        qemu_args+=(
            "-device" "virtio-serial,romfile=,id=virtio-serial0"
            "-device" "virtio-blk-pci,drive=x0,serial=vext2,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
            "-device" "virtio-blk-pci,drive=x1,serial=vexfat,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
        )
    else
        qemu_args+=(
            "-device" "virtio-serial-pci,disable-legacy=on,disable-modern=off,id=virtio-serial0${EXTRA_DEV_FLAGS}"
            "-device" "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
            "-device" "virtio-blk-pci,bus=pcie.0,addr=0x7,drive=x1,serial=vexfat,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
            "-object" "rng-random,id=rng0,filename=/dev/urandom"
            "-device" "virtio-rng-pci,bus=pcie.0,addr=0x8,disable-legacy=on,disable-modern=off,rng=rng0,event_idx=off,indirect_desc=off,queue_reset=off${EXTRA_DEV_FLAGS}"
            "-drive" "if=none,format=raw,id=nvme0n1,file=./test/initramfs/build/nvme0n1.img"
            "-device" "nvme,drive=nvme0n1,serial=nvme0n1"
        )
    fi

    if [ "$ATTACH_XFSTESTS_IMAGES" = "true" ]; then
        if [ "$SCHEME" = "tdx" ]; then
            qemu_args+=(
                "-device" "virtio-blk-pci,drive=x2,serial=vxfstest,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
                "-device" "virtio-blk-pci,drive=x3,serial=vxfsscratch,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
            )
        else
            qemu_args+=(
                "-device" "virtio-blk-pci,bus=pcie.0,addr=0x9,drive=x2,serial=vxfstest,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
                "-device" "virtio-blk-pci,bus=pcie.0,addr=0xa,drive=x3,serial=vxfsscratch,${BLK_OPTS}${EXTRA_DEV_FLAGS}"
            )
        fi
    fi

    qemu_args+=(
        "${CONSOLE_ARGS[@]}"
        "-device" "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off${NET_FEATURES}${EXTRA_DEV_FLAGS}"
        "-device" "virtio-keyboard-pci,disable-legacy=on,disable-modern=off"
    )
elif [ "$VIRTIO_BUS" = "device" ]; then
    qemu_args+=("-device" "virtio-serial-device,id=virtio-serial0")
    qemu_args+=("${CONSOLE_ARGS[@]}")
    qemu_args+=(
        "-device" "virtio-blk-device,drive=x0,serial=vext2"
        "-device" "virtio-blk-device,drive=x1,serial=vexfat"
        "-device" "virtio-net-device,netdev=net01"
        "-device" "virtio-keyboard-device"
    )

    if [ "$ATTACH_XFSTESTS_IMAGES" = "true" ]; then
        qemu_args+=(
            "-device" "virtio-blk-device,drive=x2,serial=vxfstest"
            "-device" "virtio-blk-device,drive=x3,serial=vxfsscratch"
        )
    fi
fi

if [ "$VIRTIOFS" = "on" ]; then
    echo "[$SCHEME] Enabled virtio-fs: tag=$VIRTIOFS_TAG, socket=$VIRTIOFS_SOCKET" 1>&2
    qemu_args+=(
        "-object" "memory-backend-memfd,id=mem0,size=$MEM,share=on"
        "-numa" "node,memdev=mem0"
        "-chardev" "socket,id=char0,path=$VIRTIOFS_SOCKET"
        "-device" "vhost-user-fs-pci,chardev=char0,tag=$VIRTIOFS_TAG"
    )
fi

# ==========================================================
# 6. Features
# ==========================================================

if [ "$VSOCK" = "on" ]; then
    RAND_CID=3
    echo "[$SCHEME] VSOCK enabled with CID $RAND_CID" 1>&2

    if [ "$SCHEME" = "microvm" ]; then
        qemu_args+=("-device" "vhost-vsock-device,guest-cid=$RAND_CID")
    else
        qemu_args+=("-device" "vhost-vsock-pci,id=vhost-vsock-pci0,guest-cid=$RAND_CID,disable-legacy=on,disable-modern=off${EXTRA_DEV_FLAGS}")
    fi
fi

# When using qemu-direct boot, OVMF depends on the boot protocol:
# linux-efi-* protocols require OVMF; other protocols (e.g. multiboot) do not.
if [ "$BOOT_METHOD" = "qemu-direct" ]; then
    if [ "$BOOT_PROTOCOL" = "linux-efi-pe64" ] || [ "$BOOT_PROTOCOL" = "linux-efi-handover64" ]; then
        OVMF="on"
    else
        OVMF="off"
    fi
fi

# When using `grub-rescue-iso` or `grub-qcow2` boot, OVMF must be enabled.
# Currently, the project's `grub-mkrescue` (in container image) only contained
# `x86_64-efi` platform modules — no `i386-pc`. This meant the generated ISO/qcow2
# could only be loaded by OVMF.
if [ "$BOOT_METHOD" = "grub-rescue-iso" ] || [ "$BOOT_METHOD" = "grub-qcow2" ]; then
    OVMF="on"
fi

if [ "$OVMF" = "on" ]; then
    if [ "$SCHEME" = "microvm" ]; then
        qemu_args+=("-bios" "/root/ovmf/release/microvm/MICROVM.fd")
    else
        qemu_args+=("-bios" "/root/ovmf/release/OVMF.fd")
    fi
fi

# ==========================================================
# 7. Final Output
# ==========================================================

echo "${qemu_args[*]}"
exit 0