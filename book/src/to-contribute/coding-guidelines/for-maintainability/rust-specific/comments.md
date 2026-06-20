# Comments

### Follow RFC 1574 summary line conventions (`rfc1574-summary`) {#rfc1574-summary}

The first line of a doc comment should be concise and one sentence.
Its grammatical form depends on what the item is:

- **Functions and methods** — third-person singular present indicative verb
  ("Returns", "Creates", "Acquires"), describing the action performed.
- **Types (structs, enums, traits, type aliases), modules, and fields** —
  a noun phrase naming the thing, not describing an action.
  This matches the Rust standard library convention
  (e.g., `Vec` is "A contiguous growable array type").

```rust
/// Returns the mapping's start address.
pub fn map_to_addr(&self) -> Vaddr {
    self.map_to_addr
}

/// A policy for how [`FsPath::from_fd_at`] treats an empty `path_str`.
pub enum EmptyPathStr { /* ... */ }

/// A guard that releases a [`SpinLock`] when dropped.
pub struct SpinLockGuard<'a, T> { /* ... */ }
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
