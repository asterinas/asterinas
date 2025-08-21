# OSDK User Guide

## Overview

The Asterinas OSDK (short for Operating System Development Kit) 
is designed to simplify the development of Rust operating systems.
It aims to streamline the process 
by leveraging [the framekernel architecture](../../kernel/the-framekernel-architecture.md). 

The OSDK provides a command-line tool `cargo-osdk`,
which facilitates project management 
for those developed on the framekernel architecture.
`cargo-osdk` can be used as a subcommand of Cargo.
Much like Cargo for Rust projects,
`cargo-osdk` enables building, running,
and testing projects conveniently.

## Install OSDK

### Requirements
Currently, OSDK is only supported on x86_64 Ubuntu systems.
We will add support for more operating systems in the future.

To run a kernel developed by OSDK with QEMU,
the following tools need to be installed:
- Rust >= 1.75.0
- cargo-binutils
- gcc
- gdb
- grub
- ovmf
- qemu-system-x86_64
- xorriso

The dependencies required for installing Rust and running QEMU can be installed by:
```bash
apt install build-essential curl gdb grub-efi-amd64 grub2-common \
    libpixman-1-dev mtools ovmf qemu-system-x86 xorriso
```

About how to install Rust, you can refer to
the [official site](https://www.rust-lang.org/tools/install).

`cargo-binutils` can be installed
after Rust is installed by
```bash
cargo install cargo-binutils
```

### Install

`cargo-osdk` is published on [crates.io](https://crates.io/),
and can be installed by running
```bash
cargo install cargo-osdk
```

### Upgrade
If `cargo-osdk` is already installed,
the tool can be upgraded by running
```bash
cargo install --force cargo-osdk
```
