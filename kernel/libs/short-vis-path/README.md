# short-vis-path

A proc-macro attribute for using short visibility paths in Rust.

## Background

When working with deep module hierarchies, writing `pub(in crate::very::long::path)` repeatedly can be cumbersome. This library allows you to use short identifiers in visibility restrictions, which are automatically expanded to full paths at compile time. See [Asterinas#3188](https://github.com/asterinas/asterinas/issues/3188) for the design.

### Basic Example

In a file at `src/fs/procfs.rs`:

```rust,ignore
#![short_vis_path::add(fs)]

// Expands to `pub(in crate::fs) enum E {}`
pub(in fs) enum E {}
```

This attribute uses inner attributes (`#![...]`), which requires the `custom_inner_attributes` and `proc_macro_hygiene` features. Enable them in your crate root:

```rust,ignore
// `src/lib.rs` or `src/main.rs`
#![feature(custom_inner_attributes, proc_macro_hygiene)]
```

### Multiple Identifiers

You can specify multiple identifiers in the attribute. Each identifier is matched against the current file's module path, and the longest matching segment is used:

```rust,ignore
// In `src/fs/procfs/child.rs`
#![short_vis_path::add(fs, procfs)]

// `fs` expands to `crate::fs` (matches the first segment),
// `procfs` expands to `crate::fs::procfs` (matches two segments).
pub(in fs) fn foo() {}
pub(in procfs) fn bar() {}
```

### Path Override

When there are multiple modules with the same name in the current file's path, the macro expands to the deepest matching module by default. Use path override to specify a different ancestor module:

```rust,ignore
// In `src/fs/procfs/fs/child.rs`
#![short_vis_path::add(fs = crate::fs)]

// Without override, `fs` would expand to `crate::fs::procfs::fs` (the deepest match),
// With override, `fs` expands to `crate::fs` (the ancestor module).
pub(in fs) fn foo() {}
```

## How It Works

1. The `#[short_vis_path::add(...)]` attribute is placed at the crate or module level.
2. It parses the identifiers you specify and maps them to full module paths based on the current file's location.
3. During compilation, it transforms all `pub(in short_id)` visibility restrictions to their full path equivalents.

The path is automatically inferred from the file's location in the source tree. For example:
- `src/fs/procfs.rs` → `crate::fs::procfs`
- `src/fs/procfs/mod.rs` → `crate::fs::procfs`

The Rust compiler requires that the expanded path should refer to an ancestor module; expanding to an unrelated module would result in a compilation error.

## License

MPL-2.0
