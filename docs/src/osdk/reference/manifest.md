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

The rules are:

- If running commands in the workspace root directory,
the `OSDK.toml` of the workspace will be used
- If running commands in the crate directory
  - If the crate has `OSDK.toml`,
  the `OSDK.toml` of the crate will be used.
  - Otherwise, the `OSDK.toml` of the workspace will be used.

## Configurations

Below, you will find a comprehensive version of
the available configuration options in the manifest.

```toml
[project] # <1>
type = "kernel"

[run] # <2>
kcmd_args = ["init=/bin/busybox", "path=/usr/local/bin"] # <3>
init_args = ["sh", "-l"] # <4>
initramfs="./build/initramfs.cpio.gz" # <5>
bootloader = "grub" # <6>
boot_protocol = "multiboot2" # <7>
grub-mkrescue = "/usr/bin/grub-mkrescue" # <8>
ovmf = "/usr/bin/ovmf" # <9>
qemu_path = "/usr/bin/qemu-system-x86_64" # <10>
qemu_args = [ # <11>
    "-machine q35,kernel-irqchip=split",
    "-cpu Icelake-Server,+x2apic",
    "-m 2G",
]

[test] # <2>
bootloader = "qemu"
qemu_args = [ # <10>
    "-machine q35,kernel-irqchip=split",
    "-cpu Icelake-Server,+x2apic",
    "-m 2G",
]

['cfg(arch="x86_64", scheme=microvm)'.run] # <12>
bootloader = "qemu"
qemu_args = [ # <10>
    "-machine microvm,rtc=on",
    "-cpu Icelake-Server,+x2apic",
    "-m 2G",
]
```

### Root level configurations

Fields in the given manifest marked with "1", "2" and "12" are
root level configurations.

1. The OSDK project specs `project`.

    Currently, OSDK only need to know the type of the project.
    We have two types of projects introduced with OSDK, which
    are `kernel` and `library`. An OSDK project should be a
    crate that reside in the directory of a crate or a workspace.

2. Running and testing actions settings `run` and `test`.

    These two fields describe the action that are needed to
    perform running or testing commands. The configurable values
    of the actions are described by 3~11, which specifies how
    would the OSDK invoke the VMM. The build action refers to
    the run action and smartly build anything that the run action
    need (e.g. a VM image or a kernel image with the appropriate
    format).

    Also, you can specify different actions depending on the
    scenarios. You can do that by the `cfg` feature described
    in the section [Cfg](#Cfg).

### Action configurations

3. The arguments provided will be passed to the guest kernel.

    Optional. The default value is empty.

    Each argument should be in one of the following two forms:
    `KEY=VALUE` or `KEY` if no value is required.
    Each `KEY` can appear at most once.

4. The arguments provided will be passed to the init process,
usually, the init shell.

    Optional. The default value is empty.

5. The path to the built initramfs.

    Optional. The default value is empty.

6. The bootloader used to boot the kernel.

    Optional. The default value is `grub`.

    The allowed values are `grub` and `qemu`
    (`qemu` indicates that QEMU directly boots the kernel).

7. The boot protocol used to boot the kernel.

    Optional. The default value is `multiboot2`.
    Except that for QEMU direct boot (when `bootloader` is "qemu"`),
    `multiboot` will be used.

    The allowed values are `linux-efi-handover64`,
    `linux-legacy32`, `multiboot`, and `multiboot2`.

8. The path of `grub-mkrescue`,
which is used to create a GRUB CD_ROM.

    Optional. The default value is system path,
    determined using `which grub-mkrescue`.

    This argument only takes effect
    when the bootloader is `grub`.

9. The path of OVMF. OVMF enables UEFI support for QEMU.

    Optional. The default value is empty.

    This argument only takes effect
    when the boot protocol is `linux-efi-handover64`.

10. The path of QEMU.

    Optional. If you want to use a customized QEMU this
    is the way. Otherwise we will look from the `PATH`
    environment variable for the QEMU command with appropriate
    architecture.

11. Additional arguments passed to QEMU.

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

### Cfg

Cfg is an advanced feature to create multiple profiles for
the same actions under different scenarios. Currently we
have two configurable keys, which are `arch` and `scheme`.
The key `arch` has a fixed set of values which is aligned
with the CLI `--arch` argument. If an action has no specified
arch, it matches all the architectures. The key `scheme` allows
user-defined values and can be selected by the `--scheme` CLI
argument. The key `scheme` can be used to create special settings
(especially special QEMU configurations). If a cfg action is
matched, unspecified and required arguments will be inherited
from the action that has no cfg (i.e. the default action setting).
