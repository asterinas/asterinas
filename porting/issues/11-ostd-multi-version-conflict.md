# Issue 11: Multiple versions of `ostd` in the dependency graph

## Symptom

After fixing the global allocator conflict, the build fails with a trait bound
error and Cargo notes multiple versions of `ostd`:

```text
error[E0277]: the trait bound `HeapAllocator: GlobalHeapAllocator` is not satisfied
note: there are multiple different versions of crate `ostd` in the dependency graph
  --> ostd/src/mm/heap/mod.rs:33:1
```

## Cause

`aster-kernel` depends on the local `ostd` by path, but `cargo-osdk` generates
a wrapper crate that pulls `ostd` from crates.io by version (`0.18.0`). Cargo
treats the local path dependency and the crates.io dependency as different
crates. The same problem can happen for `osdk-frame-allocator` and
`osdk-heap-allocator`.

## Fix

Use `OSDK_LOCAL_DEV=1` when installing `cargo-osdk`. This tells the installer to
use local path dependencies for all internal crates (`ostd`,
`osdk-frame-allocator`, `osdk-heap-allocator`).

```bash
cd osdk
OSDK_LOCAL_DEV=1 cargo install --path . --locked --force
```

> `OSDK_LOCAL_DEV` must be set when **building/installing** `cargo-osdk`, not
> when running it later.

## Verification

Re-run the kernel build:

```bash
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64
```

The "multiple versions of crate `ostd`" note should no longer appear.
