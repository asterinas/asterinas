# Error Handling

Rust's error handling model — `Result`, `?`, and typed errors —
is central to writing reliable kernel code.

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
