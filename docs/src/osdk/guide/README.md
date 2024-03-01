# OSDK User Guide

## Overview

OSDK (short for Operating System Development Kit) 
is designed to simplify the development of Rust operating systems.
It aims to streamline the process 
by leveraging [the framekernel architecture](../../kernel/the-framekernel-architecture.md). 

OSDK provides a command-line tool `cargo-osdk`,
which facilitates project management 
for those developed on the framekernel architecture.
`cargo-osdk` can be used as a subcommand of Cargo.
Much like Cargo for Rust projects,
`cargo-osdk` enables building, running,
and testing projects conveniently.

## Install OSDK

### Requirements
Currently, OSDK only works on x86_64 ubuntu system.
We will add support for more operating systems in the future.

To run a kernel developed by OSDK with QEMU,
the following tools need to be installed:
- Rust >= 1.75.0
- cargo-binutils
- gcc
- qemu-system-x86_64
- grub-mkrescue
- ovmf
- xorriso

About how to install Rust, you can refer to
the [official site](https://www.rust-lang.org/tools/install).

`cargo-binutils` can be installed
after Rust is installed by
```bash
cargo install cargo-binutils
```

Other tools can be installed by
```bash
apt install build-essential grub2-common qemu-system-x86 ovmf xorriso
```

### Install

`cargo-osdk` is published on [crates.io](https://crates.io/),
and can be installed by
```bash
cargo install cargo-osdk
```

### Upgrate
If `cargo-osdk` is already installed,
the tool can be upgraded by
```bash
cargo install --force cargo-osdk
```
