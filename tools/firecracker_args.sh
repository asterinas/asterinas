#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# This script generates Firecracker command-line arguments.
# Usage: `firecracker_args.sh [scheme]`
#  - scheme: "boot" (default) — boot test with a minimal initrd.
# Other arguments are configured via environmental variables:
#  - KERNEL_PATH: path to the kernel ELF binary;
#  - INITRD_PATH: path to the initramfs cpio archive;
#  - BOOT_ARGS: kernel command line arguments;
#  - VCPU: number of vCPUs (default: 2);
#  - MEM: memory size in MiB (default: 1024);
#  - FC_CONFIG_FILE: path to the generated Firecracker VM config JSON.
#    Defaults to /tmp/fc-vm.json.

SCHEME=${1:-"boot"}

KERNEL_PATH=${KERNEL_PATH:-"./target/osdk/aster-kernel/aster-kernel-osdk-bin.elf"}
INITRD_PATH=${INITRD_PATH:-"./test/initramfs/build/initramfs.cpio.gz"}
BOOT_ARGS=${BOOT_ARGS:-"console=ttyS0 earlycon loglevel=error init=/bin/echo Entered userspace"}
VCPU=${VCPU:-2}
MEM=${MEM:-1024}
FC_CONFIG_FILE=${FC_CONFIG_FILE:-/tmp/fc-vm.json}

# Generate the VM configuration JSON as a side effect.
cat > "$FC_CONFIG_FILE" << EOF
{
  "boot-source": {
    "kernel_image_path": "$KERNEL_PATH",
    "initrd_path": "$INITRD_PATH",
    "boot_args": "$BOOT_ARGS"
  },
  "drives": [],
  "machine-config": {
    "vcpu_count": $VCPU,
    "mem_size_mib": $MEM
  }
}
EOF

# Output Firecracker CLI arguments.
FC_ARGS="--no-api --config-file $FC_CONFIG_FILE"

echo $FC_ARGS
