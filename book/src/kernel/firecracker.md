# Running Asterinas on Firecracker

Asterinas can run as a guest kernel inside a
[Firecracker](https://github.com/firecracker-microvm/firecracker) microVM
through the PVH boot protocol.
This page shows how to build the kernel and boot it with Firecracker.

## Prerequisites

- An x86-64 host with KVM enabled (`/dev/kvm` accessible).
- A `firecracker` binary in `PATH`.
  The Asterinas Docker development image already has Firecracker installed.

## Build the kernel

```bash
make kernel BOOT_PROTOCOL=pvh
```

This builds a PVH-bootable kernel image.
`BOOT_PROTOCOL=pvh` automatically uses the `vmm-direct` boot method
and enables the `pvh_boot` feature.

The output kernel image is:

```text
target/osdk/asterinas/asterinas-osdk-bin.elf
```

The default initramfs used for the boot test is:

```text
test/initramfs/build/initramfs.cpio.gz
```

## Boot with Firecracker

Create a Firecracker VM configuration file, for example `vm_config.json`:

```json
{
  "boot-source": {
    "kernel_image_path": "target/osdk/asterinas/asterinas-osdk-bin.elf",
    "initrd_path": "test/initramfs/build/initramfs.cpio.gz",
    "boot_args": "console=ttyS0 rdinit=/bin/sh"
  },
  "drives": [],
  "machine-config": {
    "vcpu_count": 2,
    "mem_size_mib": 1024
  }
}
```

The `rdinit=/bin/sh` kernel parameter in `boot_args`
selects the init program to run as the first userspace process;
here it starts a BusyBox shell on the serial console.
Omitting `rdinit` boots the default initramfs `/init` instead.

Then boot the microVM:

```bash
firecracker --no-api --config-file vm_config.json
```
