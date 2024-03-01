# Manifest

## Overview

OSDK utilizes a manifest to define its precise behavior.
Typically, the configuration file is named `OSDK.toml`
and is placed in the root directory of the workspace
(the same directory as the workspace's `Cargo.toml`).
If there is only one crate and no workspace,
the file is placed in the crate's root directory.

For a crate inside workspace,
it may have two related manifests,
one is of the workspace
(in the same directory as the workspace's `Cargo.toml`)
and one of the crate
(in the same directory as the crate's `Cargo.toml`).
So which manifest should be used?

The rules are

- If running commands in the workspace root directory,
the `OSDK.toml` of the workspace will be used
- If running commands in the crate directory
  - If the crate has `OSDK.toml`,
  the `OSDK.toml` of the crate will be used.
  - Otherwise, `the OSDK.toml` of the workspace will be used.

## Configurations

Below, you will find a comprehensive version of
the available configuration options in the manifest.

```toml
kcmd_args = ["init=/bin/busybox", "path=/usr/local/bin"] # <1>
init_args = ["sh", "-l"] # <2>
initramfs="./build/initramfs.cpio.gz" # <3>

[boot]
loader = "grub" # <4>
protocol = "multiboot2" # <5>
grub-mkrescue = "/usr/bin/grub-mkrescue" # <6>
ovmf = "/usr/bin/ovmf" # <7>

[qemu]
path = "/usr/bin/qemu-system-x86_64" # <8>
machine = "q35" # <9>
args = [ # <10>
    "-enable-kvm",
    "-m 2G", 
    "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off"
] 
```

1. The arguments provided will be passed to the guest kernel.

Optional. The default value is empty.

Each argument should be in one of the following two forms:
`KEY=VALUE` or `KEY` if no value is required.
Each `KEY` can appear at most once.

2. The arguments provided will be passed to the init process,
usually, the init shell.

Optional. The default value is empty.

3. The path to the built initramfs.

Optional. The default value is empty.

4. The bootloader used to boot the kernel.

Optional. The default value is `grub`.

The allowed values are `grub` and `qemu`
(`qemu` indicates that QEMU directly boots the kernel).

5. The boot protocol used to boot the kernel.

Optional. The default value is `multiboot2`.

The allowed values are `linux-efi-handover64`,
`linux-legacy32`, `multiboot`, and `multiboot2`.

6. The path of `grub-mkrescue`,
which is used to create a GRUB CD_ROM.

Optional. The default value is system path,
determined using `which grub-mkrescue`.

This argument only takes effect
when the bootloader is `grub`.

7. The path of OVMF. OVMF enables UEFI support for QEMU.

Optional. The default value is empty.

This argument only takes effect
when the boot protocol is `linux-efi-handover64`.

8. The path of QEMU.

Optional. The default value is system path,
determined using `which qemu-system-x86_64`.

9. The machine type of QEMU.

Optional. Default is `q35`.

The allowed values are `q35` and `microvm`.

10. Additional arguments passed to QEMU.

Optional. The default value is empty.

Each argument should be in the form `KEY VALUE`
(separated by space),
or `KEY` if no value is required.
Some keys can appear multiple times
(e.g., `-device`, `-netdev`),
while other keys can appear at most once.
Certain keys, such as `-cpu` and `-machine`,
are not allowed to be set here
as they may conflict with the internal settings of OSDK.
