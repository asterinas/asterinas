# osdk-frame-allocator

This is the default buddy system frame allocator shipped with
[OSDK](https://crates.io/crates/cargo-osdk). It relies on the physical frame
metadata system in [OSTD](https://crates.io/crates/ostd) to provide a heap-free
implementation of a buddy system allocator for OS kernels. It also features
per-CPU caches and pools for scalable allocations.

This crate is part of the [Asterinas](https://github.com/asterinas/asterinas)
project.
