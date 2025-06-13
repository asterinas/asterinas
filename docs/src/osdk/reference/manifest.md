# Manifest

## Overview

The OSDK tool utilizes a manifest to define its precise behavior.
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
OSDK will firstly try to find the crate-level manifest.
If the crate-level manifest is found, OSDK uses it only.
If the manifest is not found, OSDK will look into the
workspace-level manifest.

## Configurations

Below, you will find a comprehensive version of
the available configurations in the manifest.

```toml
project_type = "kernel"                     # <1> 

# --------------------------- the default scheme settings -------------------------------
supported_archs = ["x86_64", "riscv64"]     # <2>

# The common options for all build, run and test subcommands 
[build]                                     # <3>
features = ["no_std", "alloc"]              # <4>
profile = "dev"                             # <5>
strip_elf = false                           # <6>
encoding = "raw"                            # <7>
[boot]                                      # <8>
method = "qemu-direct"                      # <9>
kcmd_args = ["SHELL=/bin/sh", "HOME=/"]     # <10>
init_args = ["sh", "-l"]                    # <11>
initramfs = "path/to/it"                    # <12>
[grub]                                      # <13>  
mkrescue_path = "path/to/it"                # <14>
protocol = "multiboot2"                     # <15> 
display_grub_menu = false                   # <16>
[qemu]                                      # <17>
path = "path/to/it"                         # <18>
args = "-machine q35 -m 2G"                 # <19>

# Special options for run subcommand
[run]                                       # <20>
[run.build]                                 # <3>
[run.boot]                                  # <8>
[run.grub]                                  # <13>
[run.qemu]                                  # <17>

# Special options for test subcommand
[test]                                      # <21>
[test.build]                                # <3>
[test.boot]                                 # <8>
[test.grub]                                 # <13>
[test.qemu]                                 # <17>
# ----------------------- end of the default scheme settings ----------------------------

# A customized scheme settings
[scheme."custom"]                           # <22>
[scheme."custom".build]                     # <3>
[scheme."custom".run]                       # <20>
[scheme."custom".test]                      # <21>
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

    Possible values are `dev`, `release`, `test`, and `bench` 
    and other profiles defined in `Cargo.toml`.

6. Whether to strip the built kernel ELF using `rust-strip`.

    Optional. The default value is `false`.

7. Denote the encoding format for kernel self-decompression

    Optional. The default value is `raw`.

    Possible values are `raw`, `gzip` and `zlib`.

    If the boot protocol is not `linux`, it is not allowed to specipy the econding format.

8. Options for booting the kernel.

9. The boot method.

    Optional. The default value is `qemu-direct`.

    Possible values are `grub-rescue-iso`, `grub-qcow2` and `qemu-direct`.

10. The arguments provided will be passed to the guest kernel.

    Optional. The default value is empty.

    Each argument should be in one of the following two forms:
    `KEY=VALUE` or `KEY` if no value is required.
    Each `KEY` can appear at most once.

11. The arguments provided will be passed to the init process,
usually, the init shell.

    Optional. The default value is empty.

12. The path to the initramfs.

    Optional. The default value is empty.

    If the path is relative, it is relative to the manifest's enclosing directory.

13. Grub options. Only take effect if boot method is `grub-rescue-iso` or `grub-qcow2`.

14. The path to the `grub-mkrescue` executable.

    Optional. The default value is the executable in the system path, if any.

    If the path is relative, it is relative to the manifest's enclosing directory.

15. The protocol GRUB used.

    Optional. The default value is `multiboot2`.

    Possible values are `linux`, `multiboot`, `multiboot2`.

16. Whether to display the GRUB menu when booting with GRUB.

    Optional. The default value is `false`.

17. Options for finding and starting QEMU.

18. The path to the QEMU executable.

    Optional. The default value is the executable in the system path, if any.

    If the path is relative, it is relative to the manifest's enclosing directory.

19. Additional arguments passed to QEMU are organized in a single string that
can include any POSIX shell compliant separators.

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

20. Special settings for running. Only take effect when running `cargo osdk run`.

    By default, it inherits common options. 
    
    Values set here are used to override common options.

21. Special settings for testing. 

    Similar to `20`, but only take effect when running `cargo osdk test`.

22. The definition of customized scheme. 

    A customized scheme has the same fields as the default scheme. 
    By default, a customized scheme will inherit all options from the default scheme,
    unless overridden by new options.

### Example

Here is a sound, self-explanatory example which is used by OSDK 
in the Asterinas project.

In the script `./tools/qemu_args.sh`, the environment variables will be
used to determine the actual set of qemu arguments.

```toml
{{#include ../../../../OSDK.toml}}
```

### Scheme

Scheme is an advanced feature that allows you to create multiple profiles for
the same action (`build`, `run`, or `test`) under different scenarios (e.g.,
x86 vs. RISC-V). Schemes support any user-defined keys (see the 22nd
configuration) and can be selected using the `--scheme` CLI argument.

If a scheme `<s>` is selected for an action (such as `test`), the value of an
unspecified but required configuration `key` will be determined by the
following rules:
 - If the general config (`scheme.<s>.key`) exists for the selected scheme, use
   this value.
 - Otherwise, if the default scheme has this configuration for the action
   (`test.key`), use this value.
 - Otherwise, if the default scheme has this configuration (`key`), use this
   value.

If the value is still not found, either the default value for the key will be
used or an error will be thrown.
