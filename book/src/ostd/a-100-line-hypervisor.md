# Example: Writing a Hypervisor in About 100 Lines of Safe Rust

This example demonstrates OSTD's hypervisor support on Intel x86 by implementing a minimal hypervisor entirely in safe Rust.

This minimal hypervisor aims at running the following Hello World assembly program in a guest.
The guest loops over the `"Hello World\n"` string and writes each character to COM1 via the `out` instruction.

```s
{{#include ../../../osdk/tests/examples_in_book/write_a_hypervisor_in_100_lines_templates/guest_hello.S}}
```

[RFC-0003](../rfcs/0003-hypervisor-support.md) describes the OSTD support required by this minimal hypervisor.
Three core OSTD abstractions provide the kernel with hypervisor capabilities:

- `GuestPhysMemSpace` maps guest physical addresses to OSTD-managed frames using EPT.
- `GuestContext` provides access to the guest CPU state.
- `GuestMode::execute` enters or resumes the guest until a VM exit or kernel event needs handling.

The code of this minimal hypervisor is shown below:

```rust
{{#include ../../../osdk/tests/examples_in_book/write_a_hypervisor_in_100_lines_templates/lib.rs}}
```
