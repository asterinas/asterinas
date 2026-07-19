# High-level Kernel Components

This directory is reserved for high-level kernel components that depend on
services exported by `aster-core`. It currently contains no component crate,
and no generic assembler-level component selection or wiring mechanism is
implemented.

Dependencies must point down the kernel crate graph. A high-level component may
depend on `aster-core` and lower-level crates, but those crates must not depend
on the component by name. When lower-level code needs behavior implemented by a
high-level component, the interaction must use an interface owned by a lower
layer.

See [Components](../../book/src/kernel/the-approach/components.md) for the
component model, initialization mechanism, and current limitations.
