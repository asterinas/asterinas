# Creating an OS Project

The OSDK can be used to create a new kernel project
or a new library project.
A kernel project defines the entry point of the kernel
and can be run with QEMU.
A library project can provide certain OS functionalities
and be imported by other OSes.

## Creating a new kernel project

Creating a new kernel project is simple.
You only need to execute the following command:

```bash
cargo osdk new --kernel myos
```

## Creating a new library project

Creating a new library project requires just one command:

```bash
cargo osdk new mylib
```

## Generated files

Next, we will introduce 
the contents of the generated project in detail.
If you don't wish to delve into the details,
you can skip the following sections.

### Overview

The generated directory
for both the kernel project and library project
contains the following contents:

```text
myos/
├── Cargo.toml
├── OSDK.toml
├── rust-toolchain.toml
└── src/
    └── lib.rs
```

### `src/lib.rs`

#### Kernel project

The `src/lib.rs` file contains the code for a simple kernel.
The function marked with the `#[ostd::main]` macro
is considered the kernel entry point by OSDK.
The kernel 
will print `Hello world from the guest kernel!`to the console 
and then abort.

```rust
{{#include ../../../../osdk/src/commands/new/kernel.template}}
```

#### Library project

The `src/lib.rs` of library project only contains
a simple kernel mode unit test.
It follows a similar code pattern as user mode unit tests.
The test module is marked with the `#[cfg(ktest)]` macro,
and each test case is marked with `#[ktest]`.

```rust
{{#include ../../../../osdk/src/commands/new/lib.template}}
```

### `Cargo.toml`

The `Cargo.toml` file is the Rust project manifest.
In addition to the contents of a normal Rust project,
OSDK will add the dependencies of the Asterinas OSTD to the file.
The dependency version may change over time.

```toml
[dependencies.ostd]
git = "https://github.com/asterinas/asterinas"
branch = "main"
```

OSDK will also exclude the directory 
which is used to generate temporary files.
```toml
[workspace]
exclude = ["target/osdk/base"]
```

### `OSDK.toml`

The `OSDK.toml` file is a manifest
that defines the exact behavior of OSDK.
By default, it includes settings on how to start QEMU to run a kernel.
The meaning of each key can be found
in the [manifest documentation](../reference/manifest.md).
Please avoid changing the default settings
unless you know what you are doing.

The default manifest of a kernel project:

```toml
{{#include ../../../../osdk/src/commands/new/kernel.OSDK.toml.template}}
```

### `rust-toolchain.toml`

The Rust toolchain for the kernel.
It is the same as the toolchain of the Asterinas OSTD.
