# Issue 06: `let` expressions unstable in `aster-time`

## Symptom

```text
error[E0658]: `let` expressions in this position are unstable
  --> kernel/comps/time/src/rtc/cmos.rs:98:15
   |
98 |         while let new = Self::from_rtc_raw(century_register) && now != new {
   |               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

## Cause

The `let_chains` feature is used in `kernel/comps/time/src/rtc/cmos.rs` but the
feature gate is not enabled in the crate.

## Fix

Add the feature gate to `kernel/comps/time/src/lib.rs`:

```rust
#![feature(let_chains)]
```

## Verification

`cargo osdk build --scheme riscv --target-arch riscv64` compiles `aster-time`
successfully.

> This fix is already applied in this repository.
