# Correctness

### Use checked or saturating arithmetic (`checked-arithmetic`) {#checked-arithmetic}

Use checked or saturating arithmetic
for operations that could overflow.
Prefer explicit overflow handling
over silent wrapping:

```rust
// Good — overflow is handled explicitly
let total = base.checked_add(offset)
    .ok_or(Error::new(Errno::EOVERFLOW))?;

// Good — clamps instead of wrapping
let remaining = budget.saturating_sub(cost);

// Bad — may silently wrap in release builds
let total = base + offset;
```

If wraparound behavior is intentional,
use explicit `wrapping_*` or `overflowing_*` operations
and document why wrapping is correct.

### Use `debug_assert` for correctness-only checks (`debug-assert`) {#debug-assert}

Assertions verifying invariants
that should never fail in correct code
belong in `debug_assert!`, not `assert!`.
`debug_assert!` is compiled out in release builds,
so the check catches bugs during development
without costing anything in production.

```rust
debug_assert!(self.align.is_multiple_of(PAGE_SIZE));
debug_assert!(self.align.is_power_of_two());
```

See also:
[std::debug_assert!](https://doc.rust-lang.org/std/macro.debug_assert.html)
and [Rust Reference: `debug_assertions`](https://doc.rust-lang.org/reference/conditional-compilation.html#debug_assertions).

### Propagate errors with `?` (`propagate-errors`) {#propagate-errors}

Use the `?` operator
to propagate errors idiomatically.
In kernel code,
`.unwrap()` is rejected
wherever failure is a legitimate possibility.

```rust
// Good — propagate with ?
let tsc_info = cpuid.get_tsc_info()?;
let frequency = tsc_info.nominal_frequency()?;

// Bad — unwrap hides the failure path
let tsc_info = cpuid.get_tsc_info().unwrap();
```

See also:
_The Rust Programming Language_, Chapter 9 "Error Handling"
and [Rust by Example: unpacking options and defaults with `?`](https://doc.rust-lang.org/rust-by-example/std/result/question_mark.html).

### Match Linux syscall semantics (`match-linux-semantics`) {#match-linux-semantics}

Syscall implementations must match Linux's return values,
error codes, and side effects for supported argument combinations.
A recognized flag may still be invalid in a particular branch.

When accepting new flags or argument values,
recheck every affected branch, including unchanged ones.
Keep null pointers, empty strings, and sentinel values distinct
when Linux assigns them different meanings.

For example, `utimensat`'s null-pathname update branch
must reject nonzero flags:

```rust
// Good — reject invalid flags before resolving the fd-only target
if !flags.is_empty() {
    return_errno_with_message!(Errno::EINVAL, "flags require a pathname");
}
let fs_path = FsPath::from_fd(dirfd)?;

// Bad — resolve the fd-only target without checking its flags
let fs_path = FsPath::from_fd(dirfd)?;
```

See also:
PR [#3176](https://github.com/asterinas/asterinas/pull/3176#discussion_r3137887431).

### Prevent reachable kernel panics (`reachable-panic`) {#reachable-panic}

Kernel code must handle user input, recoverable errors,
and valid state changes without panicking.
Use `unwrap()`, `expect()`, indexing, or assertions
only when their preconditions hold on every reachable path.
Check caller validation and helper side effects:
a merge can remove a saved key even while holding a lock.

```rust
// Good — a prior merge has already completed this entry's update.
let Some(entry) = entries.remove(&key) else {
    continue;
};

// Bad — an earlier iteration may have merged this entry away.
let entry = entries.remove(&key).unwrap();
```

Skip a missing entry only if its work is already complete;
otherwise return an error or recompute the remaining work.

See also:
the [fix for a panic after merging adjacent mappings](https://github.com/asterinas/asterinas/commit/c7b633e9bdf425d7aa3f7a5a5bb4fe819517da01).
