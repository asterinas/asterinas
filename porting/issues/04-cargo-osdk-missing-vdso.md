# Issue 04: `cargo osdk build` fails with missing `VDSO_LIBRARY_DIR`

## Symptom

```text
error: environment variable `VDSO_LIBRARY_DIR` not defined at compile time
   --> kernel/src/vdso.rs:228:28
```

## Cause

Asterinas needs a pre-built Linux vDSO library. It is provided by the separate
`asterinas/linux_vdso` repository and is not vendored in this tree.

## Fix

Clone the vDSO repository and point the build at it:

```bash
cd ~
git clone https://github.com/asterinas/linux_vdso
export VDSO_LIBRARY_DIR=$HOME/linux_vdso
cargo osdk build --scheme riscv --target-arch riscv64
```

## Verification

The build proceeds past `kernel/src/vdso.rs` without the environment-variable
error.
