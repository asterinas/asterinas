# Running Asterinas on Firecracker

Asterinas can run as a guest kernel inside a
[Firecracker](https://github.com/firecracker-microvm/firecracker) microVM
through the PVH boot protocol.
This page shows how to build the kernel and boot it with Firecracker.

## Prerequisites

- An x86-64 host with KVM enabled (`/dev/kvm` accessible).
- A `firecracker` binary in `PATH`.
  The Asterinas Docker development image already has Firecracker installed.
  If you need to install it again or are not using the image,
  download the binary from the
  [Firecracker releases](https://github.com/firecracker-microvm/firecracker/releases) page.

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

The script `tools/firecracker_args.sh` generates a Firecracker VM configuration
and prints the command-line arguments for `firecracker`.

It supports two schemes:

- `boot` (default): boot with the default kernel command line.
- `ci-boot`: which is used by CI to detect a successful boot.

Run it like this:

```bash
firecracker $(./tools/firecracker_args.sh)
```

The script accepts the following environment variables:

| Variable        | Default                                             | Description                     |
| --------------- | --------------------------------------------------- | ------------------------------- |
| `KERNEL_PATH`   | `./target/osdk/asterinas/asterinas-osdk-bin.elf`    | Path to the kernel ELF          |
| `INITRD_PATH`   | `./test/initramfs/build/initramfs.cpio.gz`          | Path to the initramfs cpio/gz   |
| `BOOT_ARGS`     | `console=ttyS0 earlycon loglevel=error i8042.exist` | Kernel command line             |
| `VCPU`          | `2`                                                 | Number of vCPUs                 |
| `MEM`           | `1024`                                              | Memory size in MiB              |
