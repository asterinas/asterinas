#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# This script is used to generate QEMU arguments for OSDK.
# Usage: `qemu_args.sh [scheme]`
#  - scheme: "normal", "microvm" or "iommu";
# Other arguments are configured via environmental variables:
#  - OVMF: "on" or "off";
#  - NETDEV: "user" or "tap";
#  - VHOST: "off" or "on";
#  - VSOCK: "off" or "on";
#  - SMP: number of CPUs;
#  - MEM: amount of memory, e.g. "8G".

OVMF=${OVMF:-"on"}
VHOST=${VHOST:-"off"}
VSOCK=${VSOCK:-"off"}
NETDEV=${NETDEV:-"user"}

SSH_RAND_PORT=${SSH_PORT:-$(shuf -i 1024-65535 -n 1)}
NGINX_RAND_PORT=${NGINX_PORT:-$(shuf -i 1024-65535 -n 1)}
REDIS_RAND_PORT=${REDIS_PORT:-$(shuf -i 1024-65535 -n 1)}
IPERF_RAND_PORT=${IPERF_PORT:-$(shuf -i 1024-65535 -n 1)}
LMBENCH_TCP_LAT_RAND_PORT=${LMBENCH_TCP_LAT_PORT:-$(shuf -i 1024-65535 -n 1)}
LMBENCH_TCP_BW_RAND_PORT=${LMBENCH_TCP_BW_PORT:-$(shuf -i 1024-65535 -n 1)}
MEMCACHED_RAND_PORT=${MEMCACHED_PORT:-$(shuf -i 1024-65535 -n 1)}

# Optional QEMU arguments. Opt in them manually if needed.
# QEMU_OPT_ARG_DUMP_PACKETS="-object filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap"

if [ "$NETDEV" = "user" ]; then
    echo "[$1] Forwarded QEMU guest port: $SSH_RAND_PORT->22; $NGINX_RAND_PORT->8080 $REDIS_RAND_PORT->6379 $IPERF_RAND_PORT->5201 $LMBENCH_TCP_LAT_RAND_PORT->31234 $LMBENCH_TCP_BW_RAND_PORT->31236 $MEMCACHED_RAND_PORT->11211" 1>&2
    NETDEV_ARGS="-netdev user,id=net01,hostfwd=tcp::$SSH_RAND_PORT-:22,hostfwd=tcp::$NGINX_RAND_PORT-:8080,hostfwd=tcp::$REDIS_RAND_PORT-:6379,hostfwd=tcp::$IPERF_RAND_PORT-:5201,hostfwd=tcp::$LMBENCH_TCP_LAT_RAND_PORT-:31234,hostfwd=tcp::$LMBENCH_TCP_BW_RAND_PORT-:31236,hostfwd=tcp::$MEMCACHED_RAND_PORT-:11211"
    VIRTIO_NET_FEATURES=",mrg_rxbuf=off,ctrl_rx=off,ctrl_rx_extra=off,ctrl_vlan=off,ctrl_vq=off,ctrl_guest_offloads=off,ctrl_mac_addr=off,event_idx=off,queue_reset=off,guest_announce=off,indirect_desc=off"
elif [ "$NETDEV" = "tap" ]; then
    THIS_SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
    QEMU_IFUP_SCRIPT_PATH=$THIS_SCRIPT_DIR/net/qemu-ifup.sh
    QEMU_IFDOWN_SCRIPT_PATH=$THIS_SCRIPT_DIR/net/qemu-ifdown.sh
    NETDEV_ARGS="-netdev tap,id=net01,script=$QEMU_IFUP_SCRIPT_PATH,downscript=$QEMU_IFDOWN_SCRIPT_PATH,vhost=$VHOST"
    VIRTIO_NET_FEATURES=",csum=off,guest_csum=off,ctrl_guest_offloads=off,guest_tso4=off,guest_tso6=off,guest_ecn=off,guest_ufo=off,host_tso4=off,host_tso6=off,host_ecn=off,host_ufo=off,mrg_rxbuf=off,ctrl_vq=off,ctrl_rx=off,ctrl_vlan=off,ctrl_rx_extra=off,guest_announce=off,ctrl_mac_addr=off,host_ufo=off,guest_uso4=off,guest_uso6=off,host_uso=off"
else 
    echo "Invalid netdev" 1>&2
    NETDEV_ARGS="-nic none"
fi

COMMON_QEMU_ARGS="\
    -cpu Icelake-Server,+x2apic \
    -smp ${SMP:-1} \
    -m ${MEM:-8G} \
    --no-reboot \
    -nographic \
    -display gtk \
    -serial chardev:mux \
    -monitor chardev:mux \
    -chardev stdio,id=mux,mux=on,signal=off,logfile=qemu.log \
    $NETDEV_ARGS \
    $QEMU_OPT_ARG_DUMP_PACKETS \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -drive if=none,format=raw,id=x0,file=./test/build/ext2.img \
    -drive if=none,format=raw,id=x1,file=./test/build/exfat.img \
"

if [ "$1" = "iommu" ]; then
    if [ "$OVMF" = "off" ]; then
        echo "Warning: OVMF is off, enabling it for IOMMU support." 1>&2
        OVMF="on"
    fi
    IOMMU_DEV_EXTRA=",iommu_platform=on,ats=on"
    IOMMU_EXTRA_ARGS="\
        -device intel-iommu,intremap=on,device-iotlb=on \
        -device ioh3420,id=pcie.0,chassis=1 \
    "
    # TODO: Add support for enabling IOMMU on AMD platforms
fi

QEMU_ARGS="\
    $COMMON_QEMU_ARGS \
    -machine q35,kernel-irqchip=split \
    -device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off$IOMMU_DEV_EXTRA \
    -device virtio-blk-pci,bus=pcie.0,addr=0x7,drive=x1,serial=vexfat,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off$IOMMU_DEV_EXTRA \
    -device virtio-keyboard-pci,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off$VIRTIO_NET_FEATURES$IOMMU_DEV_EXTRA \
    -device virtio-serial-pci,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtconsole,chardev=mux \
    -device virtio-gpu -vga none\
    $IOMMU_EXTRA_ARGS \
"

MICROVM_QEMU_ARGS="\
    $COMMON_QEMU_ARGS \
    -machine microvm,rtc=on \
    -nodefaults \
    -no-user-config \
    -device virtio-blk-device,drive=x0,serial=vext2 \
    -device virtio-blk-device,drive=x1,serial=vexfat \
    -device virtio-keyboard-device \
    -device virtio-net-device,netdev=net01 \
    -device virtio-serial-device \
    -device virtconsole,chardev=mux \
    -device virtio-gpu \
"

if [ "$VSOCK" = "on" ]; then
    # RAND_CID=$(shuf -i 3-65535 -n 1)
    RAND_CID=3
    echo "[$1] Launched QEMU VM with CID $RAND_CID" 1>&2
    if [ "$1" = "microvm" ]; then
        MICROVM_QEMU_ARGS="
            $MICROVM_QEMU_ARGS \
            -device vhost-vsock-device,guest-cid=$RAND_CID \
        "
    else
        QEMU_ARGS="
            $QEMU_ARGS \
            -device vhost-vsock-pci,id=vhost-vsock-pci0,guest-cid=$RAND_CID,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
        "
    fi
fi


if [ "$1" = "microvm" ]; then
    QEMU_ARGS=$MICROVM_QEMU_ARGS
    echo $QEMU_ARGS
    exit 0
fi

if [ "$OVMF" = "on" ]; then
    if [ "$1" = "test" ]; then
        echo "We use QEMU direct boot for testing, which does not support OVMF, ignoring OVMF" 1>&2
    else
        OVMF_PATH="/usr/share/OVMF"
        QEMU_ARGS="${QEMU_ARGS} \
            -drive if=pflash,format=raw,unit=0,readonly=on,file=$OVMF_PATH/OVMF_CODE.fd \
            -drive if=pflash,format=raw,unit=1,file=$OVMF_PATH/OVMF_VARS.fd \
        "
    fi
fi

echo $QEMU_ARGS
