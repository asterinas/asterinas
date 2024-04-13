# cargo osdk build

## Overview

The `cargo osdk build` command is used to
compile the project and its dependencies.
The usage is as follows:

```bash
cargo osdk build [OPTIONS]
```

## Options
The options can be divided into two types:
Cargo options that can be accepted by Cargo,
and Manifest options that can also be defined
in the manifest named `OSDK.toml`.

### Cargo options

- `--profile <PROFILE>`:
Build artifacts with the specified Cargo profile
(built-in candidates are 'dev', 'release', 'test', and 'bench')
[default: dev]

- `--release`:
Build artifacts in release mode, with optimizations

- `--features <FEATURES>`:
Space or comma separated list of features to activate

### Manifest options

These options can also be defined
in the project's manifest named `OSDK.toml`.
Command line options are used to override
or append values in `OSDK.toml`.
The allowed values for each option can be found
in the [Manifest Documentation](../manifest.md).

- `--kcmd-args <ARGS>`:
Command line arguments for the guest kernel
- `--init-args <ARGS>`:
Command line arguments for the init process
- `--initramfs <PATH>`:
Path of the initramfs
- `--boot-method <METHOD>`:
The method to boot the kernel
- `--grub-boot-protocol <PROTOCOL>`:
The boot protocol for booting the kernel
- `--display-grub-menu`:
To display the GRUB menu if booting with GRUB
- `--qemu-path <PATH>`:
Path of QEMU
- `--qemu-args <ARGS>`:
Extra arguments for running QEMU

## Examples

- Build a project with `./initramfs.cpio.gz`
as the initramfs and `multiboot2` as the boot protocol used by GRUB:

```bash
cargo osdk build --initramfs="./initramfs.cpio.gz" --grub-boot-protocol="multiboot2"
```

- Build a project and append `sh`, `-l`
to init process arguments:

```bash
cargo osdk build --init_args="sh" --init_args="-l"
```
