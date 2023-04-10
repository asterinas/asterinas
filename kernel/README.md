# Jinux Source Code

## Code organization

The codebase is organized as a number of Rust crates.

* The `jinux` crate assembles all other crates into a runnable OS kernel image.
This is the only binary crate; all other crates are libraries.
* The `jinux-frame` crate constitutes the main part of the jinux framework,
providing a minimal set of _safe_ abstractions that encapsulates _unsafe_ Rust
code to deal with hardware resources like CPU, memory, and interrupts.
* The `jinux-frame-*` crates complement `jinux-frame` by providing more _safe_ 
types, APIs, or abstractions that are useful to specific aspects of the Jinux.
* The `jinux-std` crate is Jinux's equivalent of Rust's std crate, although 
their APIs are quite different. This crate offers an extensive set of 
high-level safe APIs that are widely used throughout the OS code above the
framework (i.e., the crates described below).
* The rest of `jinux-*` crates implement most of the functionalities of Jinux, e.g., 
Linux syscall dispatching, process management, file systems, network stacks,
and device drivers.

## Privilege separation

Jinux is a _framekernel_, separating the entire OS into two halves:
the _privileged_ half (so-called "frame") and the _unprivileged_ half.
Only the privileged half is allowed to include any _unsafe_ Rust code. And 
it is the privileged half's responsibility to encapsulate the _unsafe_ Rust
code in _safe_ API so that most of the OS functionalities can be implemented 
with safe Rust in the unprivileged half.

This philosophy of privilege separationn is also reflected in the code organization.

* The privileged half consists of `jinux`, `jinux-frame`, and `jinux-frame-*` crates.
* The unprivileged half consists of `jinux-std` and the rest `jinux-*` crates.
