# General Style

### Prefer semantic line breaks (`semantic-line-breaks`) {#semantic-line-breaks}

For prose in Markdown and doc comments,
insert line breaks at semantic boundaries
so each line carries one coherent idea.
At minimum, break at sentence boundaries.
For longer sentences, also consider breaking at clause boundaries.

Semantic line breaks make diffs smaller,
reviews easier,
and merge conflicts less noisy.

As an exception,
RFC documents that are mostly read-only
can use regular paragraph wrapping.

See also:
[Semantic Line Breaks](https://sembr.org/).

### Make a crate's `README.md` its crate-level documentation (`readme-as-crate-doc`) {#readme-as-crate-doc}

A published crate's `README.md` (shown on crates.io)
and its crate-level Rust doc (shown on docs.rs)
usually carry the same content.
Keep a single source of truth:
write the `README.md`,
and include it as the crate-level doc rather than maintaining a separate copy.

```rust
#![doc = include_str!("../README.md")]
```

Write the `README.md` so it renders correctly under both a Markdown renderer and rustdoc.

See also:
[Issue #2947](https://github.com/asterinas/asterinas/issues/2947)
for the rationale, the caveats, and a template;
the [`ostd-pod`](https://github.com/asterinas/asterinas/tree/main/ostd/libs/ostd-pod) crate
for a crate that already adopts it.
