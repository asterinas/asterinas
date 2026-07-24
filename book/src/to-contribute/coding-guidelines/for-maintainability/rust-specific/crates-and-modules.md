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

Inside the `aster-kernel` crate, `pub(crate)` and `pub` are equivalent,
as the crate has no downstream consumers.
Prefer the shorter `pub`.

See also:
PR [#2951](https://github.com/asterinas/asterinas/pull/2951),
[#2605](https://github.com/asterinas/asterinas/pull/2605#discussion_r2720506912),
and [#3154](https://github.com/asterinas/asterinas/pull/3154#discussion_r3100905375).

### Restrict subsystem visibility with a short name (`short-vis-path`) {#short-vis-path}

 It's a good practice to restrict the visibility of a subsystem item to the
 subsystem itself using the [`pub(in path)`][pub-in-path] syntax. However,
 writing `pub(in crate::very::long::subsystem)` becomes verbose, reduces
 readability, and makes the code harder to maintain when renaming a subsystem.

For a kernel subsystem with such long module path, use
`#![short_vis_path::add(subsystem)]` and `pub(in subsystem)` to restrict an
item's visibility to that subsystem scope.

The attribute is placed inside the module and rewrites restricted visibility
paths from the short name to the absolute module path.

```rust
// In kernel/src/fs/fs_impls/procfs/template/dir.rs

// Good
#![short_vis_path::add(procfs)]
pub(in procfs) fn struct ProcDir {}

// Bad: the visibility is not restricted, violating the narrow-visibility guideline.
pub struct ProcDir {}

// Bad: the visibility path is too long to read.
pub(in crate::fs::fs_impls::procfs) struct ProcDir {}
```

When the restricted path is straightforward or used infrequently, keep the
original long path to avoid overusing this attribute. Only follow this
guideline when **all** of the following three conditions are met:

* the submodule depth exceeds 2 levels (i.e., the target path contains at least
  two `::` separators);

```rust
// In kernel/src/fs/utils/systree_inode.rs

// Good: `fs` is a direct submodule of root, thus it's readable already.
pub(in crate::fs) struct Dentry {}

// Bad: no need to do this.
#![short_vis_path::add(fs)]
pub(in fs) struct Dentry {}
```

* `pub(super)` and `pub(self)` are inapplicable;

```rust
// In ostd/src/mm/page_table/mod.rs

// Good
pub(super) const fn vaddr_range() {}

// Bad
#![short_vis_path::add(mm)]
pub(in mm) const fn vaddr_range() {}
```

* and the restricted visibility path is used at least 2 times.

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
