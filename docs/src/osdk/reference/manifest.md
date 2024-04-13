# Manifest

## Overview

OSDK utilizes a manifest to define its precise behavior.
Typically, the configuration file is named `OSDK.toml`
and is placed in the root directory of the workspace
(the same directory as the workspace's `Cargo.toml`).
If there is only one crate and no workspace,
the file is placed in the crate's root directory.

For a crate inside workspace,
it may have two distinct related manifests,
one is of the workspace
(in the same directory as the workspace's `Cargo.toml`)
and one of the crate
(in the same directory as the crate's `Cargo.toml`).
OSDK will firstly refer to the crate-level manifest, then
query the workspace-level manifest for undefined fields.
In other words, missing fields of the crate manifest
will inherit values from the workspace manifest.

## Configurations

Below, you will find a comprehensive version of
the available configuration tree in the manifest.

Here are notes for some fields with special value treatings:
 - `*` marks the field as "will be evaluated", that the final
value of string `"S"` will be the output of `echo "S"` using the
host's shell.
 - `+` marks the path fields. The relative paths written in the
path fields will be relative to the manifest's enclosing directory.

If values are given in the tree that's the default value inferred
if that the field is not explicitly stated.

```
project_type             # The type of the current crate. Can be lib/kernel[/module]

# --------------------------- the default schema settings ---------------------
vars = []                # List of lists. These are the env vars that will be set before
                         # any evaluation happens. The variables will be evaluated
                         # **SEQUENTIALLY**.
                         # The reserved variables:
                         #  - `OSDK_CWD`: The directory of the OSDK manifest.
build
|- features = []         # List of strings, the same as Cargo
|- profile = "dev"       # String, the same as Cargo
boot
|- method                # "grub-rescue-iso"/"qemu-direct"/"grub-qcow2"
|- kcmd_args             # <1>
|- init_args             # <2>
|- initramfs +           # The path to the initramfs
grub                     # Grub options are only needed if boot method is related to GRUB
|- mkrescue_path +       # The path to the `grub-mkrescue` executable
|- protocol              # The protocol GRUB used. "linux"/"multiboot"/"multiboot2"
|- display_grub_menu     # To display the GRUB menu when booting with GRUB
qemu
|- path +                # The path to the QEMU executable
|- args *                # String. <3>
run                      # Special settings for running, which will override default ones
|- vars                  # The run specific variables evaluate after .vars
|- build                 # Overriding .build
|- boot                  # Overriding .boot
|- grub                  # Overriding .grub
|- qemu                  # Overriding .qemu
test                     # Special settings for testing, which will override default ones
|- vars                  # The test specific variables evaluate after .vars
|- build                 # Overriding .build
|- boot                  # Overriding .boot
|- grub                  # Overriding .grub
|- qemu                  # Overriding .qemu
# ----------------------- end of the default schema settings ------------------

schema."user_custom_schema"
|- ...                   # All the other fields in the default schema. Missing but
                         # needed values will be firstly filled with the default
                         # value then the corresponding field in the default schema
```

Here are some additional notes for the fields:

1. The arguments provided will be passed to the guest kernel.

    Optional. The default value is empty.

    Each argument should be in one of the following two forms:
    `KEY=VALUE` or `KEY` if no value is required.
    Each `KEY` can appear at most once.

2. The arguments provided will be passed to the init process,
usually, the init shell.

    Optional. The default value is empty.

3. Additional arguments passed to QEMU that is organized in a single string that
can have any POSIX shell compliant separators.

    Optional. The default value is empty.

    Each argument should be in the form of `KEY` and `VALUE`
    or `KEY` if no value is required.
    Some keys can appear multiple times
    (e.g., `-device`, `-netdev`),
    while other keys can appear at most once.
    Certain keys, such as `-kernel` and `-initrd`,
    are not allowed to be set here
    as they may conflict with the settings of OSDK.

    The field will be evaluated, so it is ok to use environment variables
    in the arguments (usually for paths or conditional arguments). You can
    even use this mechanism to read from files by using command replacement
    `$(cat path/to/your/custom/args/file)`.

### Example

Here is a sound, self-explanatory example similar to our usage
of OSDK in the Asterinas project.

In the script `./tools/qemu_args.sh`, the environment variables will be
used to determine the actual set of qemu arguments.

```toml
project_type = "kernel"

vars = [
    ["SMP", "1"],
    ["MEM", "2G"],
]

[boot]
method = "grub-rescue-iso"

[run]
vars = [
    ["OVMF_PATH", "/usr/share/OVMF"],
]
boot.kcmd_args = [
    "SHELL=/bin/sh",
    "LOGNAME=root",
    "HOME=/",
    "USER=root",
    "PATH=/bin:/benchmark",
    "init=/usr/bin/busybox",
]
boot.init_args = ["sh", "-l"]
boot.initramfs = "regression/build/initramfs.cpio.gz"

[test]
boot.method = "qemu-direct"

[grub]
protocol = "multiboot2"
display_grub_menu = true

[qemu]
args = "$(./tools/qemu_args.sh)"

[scheme."microvm"]
boot.method = "qemu-direct"
vars = [
    ["MICROVM", "true"],
]
qemu.args = "$(./tools/qemu_args.sh)"

[scheme."iommu"]
supported_archs = ["x86_64"]
vars = [
    ["IOMMU_DEV_EXTRA", ",iommu_platform=on,ats=on"],
    ["IOMMU_EXTRA_ARGS", """\
        -device intel-iommu,intremap=on,device-iotlb=on \
        -device ioh3420,id=pcie.0,chassis=1\
    """],
]
qemu.args = "$(./tools/qemu_args.sh)"

[scheme."intel_tdx"]
supported_archs = ["x86_64"]
build.features = ["intel_tdx"]
vars = [
    ["MEM", "8G"],
    ["OVMF_PATH", "~/tdx-tools/ovmf"],
]
boot.method = "grub-qcow2"
grub.mkrescue_path = "~/tdx-tools/grub"
grub.protocol = "linux"
qemu.args = """\
    -accel kvm \
    -name process=tdxvm,debug-threads=on \
    -m $MEM \
    -smp $SMP \
    -vga none \
"""
```

### Scheme

Scheme is an advanced feature to create multiple profiles for
the same actions under different scenarios. Scheme allows any
user-defined keys and can be selected by the `--scheme` CLI
argument. The key `scheme` can be used to create special settings
(especially special QEMU configurations). If a scheme action is
matched, unspecified and required arguments will be inherited
from the default scheme.
