# Defensive Programming

Assertions verify invariants that must hold
for the program to be correct.
Choosing the right assertion type
balances safety against runtime cost.

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
