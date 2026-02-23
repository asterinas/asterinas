# Comments and Documentation

API documentation describes API meaning and usage
and is rendered by rustdoc.
Public APIs should be documented,
including crates, modules, structs, traits, functions, and macros.
The `#![warn(missing_docs)]` lint helps enforce this baseline.

Asterinas follows Rust community documentation conventions.
Two primary references are:
1. The rustdoc book:
   [How to write documentation](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html)
2. The Rust RFC book:
   [API Documentation Conventions](https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html#appendix-a-full-conventions-text)

### Follow RFC 1574 summary line conventions (`rfc1574-summary`) {#rfc1574-summary}

The first line of a doc comment should be
third-person singular present indicative
("Returns", "Creates", "Acquires"),
concise, and one sentence.

```rust
/// Returns the mapping's start address.
pub fn map_to_addr(&self) -> Vaddr {
    self.map_to_addr
}
```

### End sentence comments with punctuation (`comment-punctuation`) {#comment-punctuation}

If a comment line is a full sentence,
end it with proper punctuation.
This improves readability in dense code
and avoids fragmented prose.

```rust
// Good — complete sentence with punctuation.
// SAFETY: The pointer is derived from a live allocation.

// Bad — complete sentence without punctuation
// SAFETY: The pointer is derived from a live allocation
```

### Wrap identifiers in backticks (`backtick-identifiers`) {#backtick-identifiers}

Type names, method names,
and code identifiers in doc comments
should be wrapped in backticks for rustdoc rendering.
When referring to types,
prefer rustdoc links (`[TypeName]`) where possible.

```rust
/// Acquires the [`SpinLock`] and returns a guard
/// that releases the lock on [`Drop`].
///
/// Callers must not call `acquire` while holding
/// a [`RwMutex`] to avoid deadlock.
pub fn acquire(&self) -> SpinLockGuard<'_, T> { ... }
```

### Do not disclose implementation details in doc comments (`no-impl-in-docs`) {#no-impl-in-docs}

Doc comments should describe _what_ the API does
and _how to use it_,
not _how it is implemented internally_.

```rust
// Good — behavior-oriented
/// Returns the number of active connections.

// Bad — leaks implementation details
/// Returns the length of the internal `HashMap`
/// that tracks connections by socket address.
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
