# Crates & Modules

### Use workspace dependencies (`workspace-deps`) {#workspace-deps}

Always declare shared dependencies
in the workspace `[workspace.dependencies]` table
and reference them with `.workspace = true`
in member crates.

```toml
# In the workspace root Cargo.toml
[workspace.dependencies]
ostd = { version = "0.17.0", path = "ostd" }
bitflags = "2.6"

# In a member crate's Cargo.toml
[dependencies]
ostd.workspace = true
bitflags.workspace = true
```

### Add module-level documentation for major components (`module-docs`) {#module-docs}

A module file that serves as
an important kernel component
(e.g., subsystem entry point, major data structure, driver)
should begin with a `//!` comment explaining:
1. What the module does
2. The key types it exposes
3. How it relates to neighboring modules

```rust
//! Virtual memory area (VMA) management.
//!
//! This module defines [`VmMapping`] and associated types,
//! which represent contiguous regions of a process's virtual address space.
//! VMAs are managed by the [`Vmar`] tree in the parent module.
```

### Default to the narrowest visibility (`narrow-visibility`) {#narrow-visibility}

Start private,
then widen to `pub(super)`, `pub(crate)`, or `pub`
only when an actual external consumer requires it.

```rust
// Good — restricted to the parent module
pub(super) static I8042_CONTROLLER:
    Once<SpinLock<I8042Controller, LocalIrqDisabled>> = Once::new();

pub(super) fn init() -> Result<(), I8042ControllerError> {
    // ...
}

// Bad — unnecessarily wide
pub static I8042_CONTROLLER: ...
```

In `aster-core`, do not assume that `pub` and `pub(crate)` are
interchangeable. A private parent module may currently limit reachability, but
a `pub` item is intended for an API that downstream component crates can use.
Keep an item `pub(crate)` or narrower until an actual downstream consumer
requires it, then expose and document the contract deliberately.

The `asterinas` assembler is not a reusable API crate. Keep its entry point
private and do not expose its wiring as a public API.

See also the visibility review discussions in
[#2951](https://github.com/asterinas/asterinas/pull/2951#discussion_r3105011202)
and
[#2605](https://github.com/asterinas/asterinas/pull/2605#discussion_r2720506912).

### Preserve the kernel crate dependency direction (`kernel-dependency-direction`) {#kernel-dependency-direction}

Cargo dependencies must preserve an acyclic kernel crate graph. A crate may
depend on crates in the same layer or any lower layer: the assembler may depend
on high-level components and lower-layer crates; high-level components may
depend on other high-level components, `aster-core`, and lower-layer crates;
and `aster-core` may depend on low-level components, libraries, and OSTD. A
crate must not depend on a crate in a higher layer.

The high-level component layer is a rule for future migrations. The current
tree has no high-level component and no generic assembler-level selection or
wiring mechanism.

When lower-layer code needs behavior implemented above it, define the
interface and registration mechanism in the lower layer and let the higher
component register its implementation. Do not introduce a reverse Cargo
dependency.

See also:
[The Kernel's Crate Architecture](../../../../kernel/the-kernel-crate-architecture.md#dependency-and-control-flow-rules).

### Place components relative to the core (`component-placement`) {#component-placement}

When introducing or extracting a component that needs `aster-core` APIs, place
it under `kernel/comps/`.
Put an initialization-bearing component that the core consumes by name
and that needs no core API under `kernel/core/comps/`.
Put reusable code that does not participate in component initialization and
does not require `aster-core` APIs under `kernel/libs/`.

Place code directly in `aster-core` only when moving it to a higher-layer crate
would require `aster-core` or a lower-layer crate to depend on that crate by
name, and a lower-owned interface cannot reasonably invert the dependency.
Explain that requirement in the pull request.

See also:
[Components](../../../../kernel/the-approach/components.md#component-layers).

### Qualify function calls with the parent module (`qualified-fn-imports`) {#qualified-fn-imports}

When importing a free function or a static/constant
from another module,
import the **parent module** and access the item
through it (`module::function()`, `module::CONSTANT`).
Do not import free functions or statics directly by name.

This convention is recommended by
[*The Rust Programming Language*](https://doc.rust-lang.org/book/ch07-04-bringing-paths-into-scope-with-the-use-keyword.html)
and followed by the Rust compiler codebase.
It serves two purposes:

1. The call site makes it clear
   that an imported item is being used,
   not a local one.
2. The module name provides context
   that complements the item name.

```rust
// Good — module-qualified function call
use ostd::irq;

let guard = irq::disable_local();

// Good — module-qualified static access
use ostd::mm::kspace;

let base = kspace::LINEAR_MAPPING_BASE_VADDR;

// Bad — bare function name; unclear origin at call site
use ostd::irq::disable_local;

let guard = disable_local();

// Bad — bare static name; could be mistaken for a local constant
use ostd::mm::kspace::LINEAR_MAPPING_BASE_VADDR;

let base = LINEAR_MAPPING_BASE_VADDR;
```

This guideline applies to **free functions and statics/constants**.
Types, traits, and enum variants
should still be imported directly by name,
following the standard Rust convention.
