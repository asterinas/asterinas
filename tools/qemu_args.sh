#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# This script is used to generate QEMU arguments for OSDK.
# The positional argument $1 is the scheme.
# A switch "-ovmf" can be passed as an argument to enable OVMF.

RAND_PORT_NUM1=$(shuf -i 1024-65535 -n 1)
RAND_PORT_NUM2=$(shuf -i 1024-65535 -n 1)

echo "[$1] Forwarded QEMU guest port: $RAND_PORT_NUM1->22; $RAND_PORT_NUM2->8080" 1>&2

COMMON_QEMU_ARGS="\
    -cpu Icelake-Server,+x2apic \
    -smp ${SMP:-1} \
    -m ${MEM:-8G} \
    --no-reboot \
    -nographic \
    -display none \
    -serial chardev:mux \
    -monitor chardev:mux \
    -chardev stdio,id=mux,mux=on,signal=off,logfile=qemu.log \
    -netdev user,id=net01,hostfwd=tcp::$RAND_PORT_NUM1-:22,hostfwd=tcp::$RAND_PORT_NUM2-:8080 \
    -object filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -drive if=none,format=raw,id=x0,file=./regression/build/ext2.img \
    -drive if=none,format=raw,id=x1,file=./regression/build/exfat.img \
"

if [ "$1" = "iommu" ]; then
    IOMMU_DEV_EXTRA=",iommu_platform=on,ats=on"
    IOMMU_EXTRA_ARGS="\
        -device intel-iommu,intremap=on,device-iotlb=on \
        -device ioh3420,id=pcie.0,chassis=1\
    "
fi

QEMU_ARGS="\
    $COMMON_QEMU_ARGS \
    -machine q35,kernel-irqchip=split \
    -device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtio-blk-pci,bus=pcie.0,addr=0x7,drive=x1,serial=vexfat,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtio-keyboard-pci,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtio-serial-pci,disable-legacy=on,disable-modern=off$IOMMU_DEV_EXTRA \
    -device virtconsole,chardev=mux \
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
"

if [ "$1" = "microvm" ]; then
    QEMU_ARGS=$MICROVM_QEMU_ARGS
    echo $QEMU_ARGS
    exit 0
fi

if [ "$1" = "-ovmf" ] || [ "$2" = "-ovmf" ]; then
    OVMF_PATH="/usr/share/OVMF"
    QEMU_ARGS="${QEMU_ARGS}\
        -drive if=pflash,format=raw,unit=0,readonly=on,file=$OVMF_PATH/OVMF_CODE.fd \
        -drive if=pflash,format=raw,unit=1,file=$OVMF_PATH/OVMF_VARS.fd \
    "
fi

echo $QEMU_ARGS
