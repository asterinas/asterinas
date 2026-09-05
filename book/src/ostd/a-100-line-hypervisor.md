# Example: Writing a Hypervisor in About 100 Lines of Safe Rust

This example demonstrates OSTD's hypervisor support by implementing a minimal hypervisor in the kernel.

This minimal hypervisor can run the following Hello World assembly program in a guest. The guest's machine code is embedded in `GUEST_CODE` in the implementation below.

```s
{{#include ../../../osdk/tests/examples_in_book/write_a_hypervisor_in_100_lines_templates/hello.S}}
```

[RFC-0003](../rfcs/0003-hypervisor-support.md) describes the OSTD support required by this minimal hypervisor. Three core OSTD abstractions provide the kernel with hypervisor capabilities:

- `GuestPhysMemSpace` maps guest physical addresses to OSTD-managed frames using EPT.
- `GuestContext` provides access to the guest CPU state.
- `GuestMode::execute` enters or resumes the guest and returns exit information.

A sample implementation of the hypervisor in kernel in safe Rust is given below.

```rust
{{#include ../../../osdk/tests/examples_in_book/write_a_hypervisor_in_100_lines_templates/lib.rs}}
```
