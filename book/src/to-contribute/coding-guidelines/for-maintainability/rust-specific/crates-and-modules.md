# Crates & Modules

### Use workspace dependencies (`workspace-deps`) {#workspace-deps}

Always declare shared dependencies
in the workspace `[workspace.dependencies]` table
and reference them with `.workspace = true`
in member crates.

```toml
# In the workspace root Cargo.toml
[workspace.dependencies]
ostd = { path = "ostd", version = "0.17.0" }
bitflags = "2.6"

# In a member crate's Cargo.toml
[dependencies]
ostd.workspace = true
bitflags.workspace = true
```

### Keep the kernel crate graph layered and acyclic (`layered-kernel-crates`) {#layered-kernel-crates}

For modularity,
the Asterinas kernel under `kernel/` is decomposed into
an acyclic graph of kernel crates arranged in layers.
A kernel crate may depend on crates in the same layer or any lower layer.
The layers are, from highest to lowest:

1. The assembler crate (`kernel/src/`).
2. High-level component crates (`kernel/comps/`).
3. The `aster-core` crate (`kernel/core/`).
4. Low-level component crates (`kernel/core/comps/`).
5. Kernel libraries (`kernel/libs/`).

A new kernel subsystem, driver, or utility type
should be added as a separate crate outside `aster-core` by default.
Add new code directly to `aster-core` only when necessary.

All these kernel crates may depend on OSTD.

When lower-layer code needs behavior implemented above it,
define the interface and registration mechanism in the lower layer
and let the higher component register its implementation.

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

The `asterinas` assembler crate does not expose any `pub` Rust items.

### Visibility modifiers encode intent, not just effect (`encode-intent-in-vis`) {#encode-intent-in-vis}

A plain `pub` says "export me as far as possible":
along its defining module path,
the item's _effective_ visibility is capped
by every enclosing module up to the crate root
(and a `pub use` elsewhere can re-export it around that cap entirely).
In a large project like Asterinas,
a `pub` item may sit deep in the module hierarchy,
making its effective visibility hard to see locally
and fragile against visibility changes in its ancestors.

Therefore, a visibility modifier in Asterinas declares
the maximum _intended_ exposure of an item,
regardless of what its ancestors happen to allow:

* `pub` means "exported from this crate".
  Reserve it for items with an actual consumer in another crate.
* `pub(crate)`, `pub(super)`, `pub(in super::super)`, and `pub(in crate::a)`
  mean "this item must never escape the named scope",
  even when an enclosing private module already restricts it further.
  If the `path` part of a `pub(in path)` modifier is long,
  apply [`short-vis-path`](#short-vis-path).

One exception is struct fields and union fields:
a `pub` field of a visibility-restricted struct or union is acceptable,
because the field sits only a few lines from the type's own modifier,
so its real visibility remains locally obvious.

This guideline is partially enforced by the rustc lint
[`unreachable_pub`](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html#unreachable-pub),
which flags any `pub` item not actually reachable from outside its crate.

### Restrict subsystem visibility with a short name (`short-vis-path`) {#short-vis-path}

To conform with the [`narrow-visibility`](#narrow-visibility) guideline,
a common pattern is to expose Rust items inside a subsystem only to the subsystem itself.
But when the subsystem sits deep in the module hierarchy,
we have to write long visibility modifiers
like `pub(in crate::a::very::deep::subsystem)`,
which is tedious for writers and unfriendly for readers.

Instead, you can use the `short_vis_path` macro
to create a short name for the long full path of a subsystem:

```rust
// In kernel/src/a/very/deep/subsystem/lib.rs

// Good
#![short_vis_path::add(subsystem)]

pub(in subsystem) fn struct Foo {}

// Bad: violating the narrow-visibility guideline.
pub struct Foo {}

// Bad: the visibility path is too long to read.
pub(in crate::a::very::deep::subsystem) struct Foo {}
```

To avoid overusing this attribute, only follow this guideline
when **all** of the following three conditions are met:

* the submodule depth exceeds 2 levels
  (i.e., the target path contains at least two `::` separators)

```rust
// In kernel/src/fs/utils/systree_inode.rs

// Good: `fs` is a direct submodule of root, thus it's readable already.
pub(in crate::fs) struct Dentry {}

// Bad: no need to do this.
#![short_vis_path::add(fs)]
pub(in fs) struct Dentry {}
```

* `pub(super)` and `pub(self)` are inapplicable

```rust
// In ostd/src/mm/page_table/mod.rs

// Good
pub(super) const fn vaddr_range() {}

// Bad
#![short_vis_path::add(mm)]
pub(in mm) const fn vaddr_range() {}
```

* and the restricted visibility path is used at least 2 times

Refer to [#3188] for the `short-vis-path` design.

[pub-in-path]: https://doc.rust-lang.org/reference/visibility-and-privacy.html#pubin-path-pubcrate-pubsuper-and-pubself
[#3188]: https://github.com/asterinas/asterinas/issues/3188

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
