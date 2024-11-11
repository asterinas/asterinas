#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# Compile the asterinas binaries via `make build RELEASE=1` first.
ISO_ROOT=../asterinas/target/osdk/iso_root/boot/
LOG_FILE=../qemu.log
IMG_FILE=../asterinas.qcow2

VNC_PORT=${1:-42}
SSH_RAND_PORT=$(shuf -i 1024-65535 -n 1)
NGINX_RAND_PORT=$(shuf -i 1024-65535 -n 1)

# Create a new qcow2 image and partition.
rm ${IMG_FILE} -f
qemu-img create -f qcow2 ${IMG_FILE} 10G
qemu-nbd -c /dev/nbd0 ${IMG_FILE}
parted -s /dev/nbd0 \
	mklabel gpt \
	mkpart primary 1MiB 512MiB \
	set 1 esp on \
	quit

# Install grub and copy asterinas files.
mnt_dir=$(mktemp -d -t "mnt-XXXXXX")
mkfs.fat -F32 -n asterinas /dev/nbd0p1
mount /dev/nbd0p1 ${mnt_dir}
grub-install --efi-directory ${mnt_dir} --boot-directory ${mnt_dir}/boot --removable
cp -r ${ISO_ROOT}/* ${mnt_dir}/boot/
tree ${mnt_dir}
umount ${mnt_dir}
rm -rf ${mnt_dir}
qemu-nbd -d /dev/nbd0

# Emulate a normal Aliyun ECS (ecs.e-c1m2.large).
qemu-system-x86_64 \
	-machine pc-i440fx-2.1,accel=kvm -cpu host -smp 2 -m 4G \
	-bios /usr/share/qemu/OVMF.fd \
	-device piix3-usb-uhci,bus=pci.0,addr=01.2 \
	-device cirrus-vga \
	-chardev stdio,id=mux,mux=on,logfile=${LOG_FILE} \
	-device virtio-serial-pci,disable-modern=true \
	-drive file=${IMG_FILE},if=none,id=mydrive \
	-device virtio-blk-pci,disable-modern=true,drive=mydrive \
	-netdev user,id=net01,hostfwd=tcp::${SSH_RAND_PORT}-:22,hostfwd=tcp::${NGINX_RAND_PORT}-:8080 \
	-device virtio-net-pci,netdev=net01,disable-modern=true \
	-device virtio-balloon,disable-modern=true \
	-serial chardev:mux \
	-vnc :${VNC_PORT}
