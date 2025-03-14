# osdk-heap-allocator

This is the default slab-based global heap allocator shipped with
[OSDK](https://crates.io/crates/cargo-osdk). It relies on the slab mechanism in
[OSTD](https://crates.io/crates/ostd) to provide a fast, memory-efficient
implementation of a global heap allocator for OS kernels. It also features
per-CPU caches for scalable allocations.

This crate is part of the [Asterinas](https://github.com/asterinas/asterinas)
project.
