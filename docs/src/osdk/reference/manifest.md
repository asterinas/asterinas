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
the available configurations in the manifest.

```toml
project_type = "kernel"                     # <1> 

# --------------------------- the default schema settings -------------------------------
supported_archs = ["x86_64", "riscv64"]     # <2>

# The common options for all build, run and test subcommands 
[build]                                     # <3>
features = ["no_std", "alloc"]              # <4>
profile = "dev"                             # <5>
strip_elf = false                           # <6>
[boot]                                      # <7>
method = "qemu-direct"                      # <8>
kcmd_args = ["SHELL=/bin/sh", "HOME=/"]     # <9>
init_args = ["sh", "-l"]                    # <10>
initramfs = "path/to/it"                    # <11>
[grub]                                      # <12>  
mkrescue_path = "path/to/it"                # <13>
protocol = "multiboot2"                     # <14> 
display_grub_menu = false                   # <15>
[qemu]                                      # <16>
path = "path/to/it"                         # <17>
args = "-machine q35 -m 2G"                 # <18>

# Special options for run subcommand
[run]                                       # <19>
[run.build]                                 # <3>
[run.boot]                                  # <7>
[run.grub]                                  # <12>
[run.qemu]                                  # <16>

# Special options for test subcommand
[test]                                      # <20>
[test.build]                                # <3>
[test.boot]                                 # <7>
[test.grub]                                 # <12>
[test.qemu]                                 # <16>
# ----------------------- end of the default schema settings ----------------------------

# A customized schema settings
[schema."custom"]                           # <21>
[schema."custom".build]                     # <3>
[schema."custom".run]                       # <19>
[schema."custom".test]                      # <20>
```

Here are some additional notes for the fields:

1. The type of current crate.

    Optional. If not specified,
    the default value is inferred from the usage of the macro `#[ostd::main]`.
    if the macro is used, the default value is `kernel`.
    Otherwise, the default value is `library`.
    
    Possible values are `library` or `kernel`.

2. The architectures that can be supported.

    Optional. By default OSDK supports all architectures.
    When building or running,
    if not specified in the CLI,
    the architecture of the host machine will be used.

    Possible values are `aarch64`, `riscv64`, `x86_64`.

3. Options for compilation stage.

4. Cargo features to activate.

    Optional. The default value is empty.

    Only features defined in `Cargo.toml` can be added to this array.

5. Build artifacts with the specified Cargo profile.

    Optional. The default value is `dev`.

    Possible values are `dev`, `release`, `test` and `bench` 
    and other profiles defined in `Cargo.toml`.

6. Whether to strip the built kernel ELF using `rust-strip`.

    Optional. The default value is `false`.

7. Options for booting the kernel.

8. The boot method.

    Optional. The default value is `qemu-direct`.

    Possible values are `grub-rescue-iso`, `grub-qcow2` and `qemu-direct`.

9. The arguments provided will be passed to the guest kernel.

    Optional. The default value is empty.

    Each argument should be in one of the following two forms:
    `KEY=VALUE` or `KEY` if no value is required.
    Each `KEY` can appear at most once.

10. The arguments provided will be passed to the init process,
usually, the init shell.

    Optional. The default value is empty.

11. The path to the initramfs.

    Optional. The default value is empty.

    If the path is relative, it is relative to the manifest's enclosing directory.

12. Grub options. Only take effect if boot method is `grub-rescue-iso` or `grub-qcow2`.

13. The path to the `grub-mkrescue` executable.

    Optional. The default value is the executable in the system path, if any.

    If the path is relative, it is relative to the manifest's enclosing directory.

14. The protocol GRUB used.

    Optional. The default value is `multiboot2`.

    Possible values are `linux`, `multiboot`, `multiboot2`.

15. Whether to display the GRUB menu when booting with GRUB.

    Optional. The default value is `false`.

16. Options for finding and starting QEMU.

17. The path to the QEMU executable.

    Optional. The default value is the executable in the system path, if any.

    If the path is relative, it is relative to the manifest's enclosing directory.

18. Additional arguments passed to QEMU that is organized in a single string that
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

19. Special settings for running. Only take effect when running `cargo osdk run`.

    By default, it inherits common options. 
    
    Values set here are used to override common options.

20. Special settings for testing. 

    Similar to `19`, but only take effect when running `cargo osdk test`.

21. The definition of customized schema. 

    A customized schema has the same fields as the default schema. 
    By default, a customized schema will inherit all options from the default schema,
    unless overrided by new options.

### Example

Here is a sound, self-explanatory example which is used by OSDK 
in the Asterinas project.

In the script `./tools/qemu_args.sh`, the environment variables will be
used to determine the actual set of qemu arguments.

```toml
{{#include ../../../../OSDK.toml}}
```

### Scheme

Scheme is an advanced feature to create multiple profiles for
the same actions under different scenarios. Scheme allows any
user-defined keys and can be selected by the `--scheme` CLI
argument. The key `scheme` can be used to create special settings
(especially special QEMU configurations). If a scheme action is
matched, unspecified and required arguments will be inherited
from the default scheme.
