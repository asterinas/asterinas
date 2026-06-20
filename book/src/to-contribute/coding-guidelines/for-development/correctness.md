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
