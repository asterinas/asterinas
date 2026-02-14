#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# This script is used to generate QEMU arguments for OSDK.
# Usage: `qemu_args.sh [scheme]`
#  - scheme: "normal", "test", "microvm" or "iommu";
# Other arguments are configured via environmental variables:
#  - OVMF: "on" or "off";
#  - BOOT_METHOD: "qemu-direct", "grub-rescue-iso", "linux-efi-pe64" or "linux-efi-handover64";
#  - NETDEV: "user" or "tap";
#  - VHOST: "off" or "on";
#  - VSOCK: "off" or "on";
#  - CONSOLE: "hvc0" to enable virtio console;
#  - SMP: number of CPUs;
#  - MEM: amount of memory, e.g. "8G";
#  - VNC_PORT: VNC port, default is "42".

# ==========================================================
# 1. Environment & Defaults
# ==========================================================

MODE=${1:-"normal"}
OVMF=${OVMF:-"on"}
VHOST=${VHOST:-"off"}
VSOCK=${VSOCK:-"off"}
NETDEV=${NETDEV:-"user"}
CONSOLE=${CONSOLE:-"hvc0"}
SMP=${SMP:-1}
MEM=${MEM:-"8G"}
VNC_PORT=${VNC_PORT:-"42"}

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
    echo "[$MODE] Forwarded QEMU guest port: $SSH_RAND_PORT->22; $NGINX_RAND_PORT->8080 $REDIS_RAND_PORT->6379 $IPERF_RAND_PORT->5201 $LMBENCH_TCP_LAT_RAND_PORT->31234 $LMBENCH_TCP_BW_RAND_PORT->31236 $MEMCACHED_RAND_PORT->11211" 1>&2
    NET_BACKEND_ARGS=(
        "-netdev" "user,id=net01,hostfwd=tcp::$SSH_RAND_PORT-:22,hostfwd=tcp::$NGINX_RAND_PORT-:8080,hostfwd=tcp::$REDIS_RAND_PORT-:6379,hostfwd=tcp::$IPERF_RAND_PORT-:5201,hostfwd=tcp::$LMBENCH_TCP_LAT_RAND_PORT-:31234,hostfwd=tcp::$LMBENCH_TCP_BW_RAND_PORT-:31236,hostfwd=tcp::$MEMCACHED_RAND_PORT-:11211"
    )
    # Common features for User networking
    NET_FEATURES=",mrg_rxbuf=off,ctrl_rx=off,ctrl_rx_extra=off,ctrl_vlan=off,ctrl_vq=off,ctrl_guest_offloads=off,ctrl_mac_addr=off,event_idx=off,queue_reset=off,guest_announce=off,indirect_desc=off"
elif [ "$NETDEV" = "tap" ]; then
    THIS_SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    NET_BACKEND_ARGS=(
        "-netdev" "tap,id=net01,script=$THIS_SCRIPT_DIR/net/qemu-ifup.sh,downscript=$THIS_SCRIPT_DIR/net/qemu-ifdown.sh,vhost=$VHOST"
    )
    # Common features for Tap networking
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

# --- Block Device Options ---
BLK_OPTS="disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off"

# ==========================================================
# 3. Mode Selection & Construction
# ==========================================================

# Helper variables for device types (can be overridden by specific modes)
VIRTIO_BUS="pci"
MACHINE_TYPE="q35"
CPU_MODEL="Icelake-Server,+x2apic"
EXTRA_DEV_FLAGS="" # Used for IOMMU

case "$MODE" in
    "tdx")
        TDX_OBJECT='{ "qom-type": "tdx-guest", "id": "tdx0", "sept-ve-disable": true, "quote-generation-socket": { "type": "vsock", "cid": "2", "port": "4050" } }'

        qemu_args+=(
            "-machine" "q35,kernel-irqchip=split,confidential-guest-support=tdx0"
            "-bios" "/root/ovmf/release/OVMF.fd"
            "-cpu" "host,-kvm-steal-time,pmu=off"
            "-object" "'$TDX_OBJECT'"
            "-vga" "none"
            "-nodefaults"
            "-device" "virtio-serial,romfile="
        )

        # In TDX, we override the standard OVMF logic (BIOS is set above)
        OVMF="off_handled"
        ;;

    "microvm")
        VIRTIO_BUS="device" # MicroVM uses MMIO devices, not PCI
        MACHINE_TYPE="microvm,rtc=on"
        OVMF="off"

        qemu_args+=(
            "-machine" "$MACHINE_TYPE"
            "-nodefaults"
            "-no-user-config"
            "-cpu" "$CPU_MODEL"
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
            "-cpu" "$CPU_MODEL"
            "-device" "intel-iommu,intremap=on,device-iotlb=on"
            "-device" "ioh3420,id=pcie.0,chassis=1"
        )
        ;;

    *)
        qemu_args+=(
            "-machine" "q35,kernel-irqchip=split"
            "-cpu" "$CPU_MODEL"
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
    "-nographic"
    "-monitor" "pty"
    "-monitor" "chardev:mux"
    "-chardev" "stdio,id=mux,mux=on,logfile=qemu.log"
    "-device" "isa-debug-exit,iobase=0xf4,iosize=0x04"
)

qemu_args+=("${NET_BACKEND_ARGS[@]}")
qemu_args+=("${DRIVE_BACKEND_ARGS[@]}")

# ==========================================================
# 5. Device Attachment
# ==========================================================

if [ "$VIRTIO_BUS" = "pci" ]; then
    if [ "$MODE" = "tdx" ]; then
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
        )
    fi

    qemu_args+=(
        "${CONSOLE_ARGS[@]}"
        "-device" "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off${NET_FEATURES}${EXTRA_DEV_FLAGS}"
        "-device" "virtio-keyboard-pci,disable-legacy=on,disable-modern=off"
    )

elif [ "$VIRTIO_BUS" = "device" ]; then
    # MMIO Devices (MicroVM)
    qemu_args+=("-device" "virtio-serial-device,id=virtio-serial0")
    qemu_args+=("${CONSOLE_ARGS[@]}")

    qemu_args+=(
        "-device" "virtio-blk-device,drive=x0,serial=vext2"
        "-device" "virtio-blk-device,drive=x1,serial=vexfat"
        "-device" "virtio-net-device,netdev=net01"
        "-device" "virtio-keyboard-device"
    )
fi

# ==========================================================
# 6. Features
# ==========================================================

# --- VSOCK ---
if [ "$VSOCK" = "on" ]; then
    # RAND_CID=$(shuf -i 3-65535 -n 1)
    RAND_CID=3
    echo "[$MODE] VSOCK enabled with CID $RAND_CID" 1>&2

    if [ "$MODE" = "microvm" ]; then
         qemu_args+=("-device" "vhost-vsock-device,guest-cid=$RAND_CID")
    else
         qemu_args+=("-device" "vhost-vsock-pci,id=vhost-vsock-pci0,guest-cid=$RAND_CID,disable-legacy=on,disable-modern=off${EXTRA_DEV_FLAGS}")
    fi
fi

# --- OVMF (UEFI) ---
if [ "$OVMF" = "on" ]; then
    if [ "$BOOT_METHOD" = "qemu-direct" ]; then
        echo "Warning: QEMU direct boot is incompatible with OVMF, ignoring OVMF." 1>&2
    else
        OVMF_PATH="/root/ovmf/release"
        qemu_args+=(
            "-drive" "if=pflash,format=raw,unit=0,readonly=on,file=$OVMF_PATH/OVMF_CODE.fd"
            "-drive" "if=pflash,format=raw,unit=1,file=$OVMF_PATH/OVMF_VARS.fd"
        )
    fi
fi

# ==========================================================
# 7. Final Output
# ==========================================================

# Output the array elements separated by spaces
echo "${qemu_args[*]}"
exit 0
