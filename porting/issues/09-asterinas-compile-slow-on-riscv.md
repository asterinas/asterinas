# Issue 09: Asterinas compilation on RISC-V is very slow

## Symptom

`cargo osdk build` runs for a long time with little or no terminal output. It
can look like the build is stuck, especially on the first run.

## Cause

- Asterinas has many dependencies; the first build downloads and compiles a
  large number of crates.
- `compiler_builtins`, `core`, and other low-level crates are expensive to
  compile for RISC-V.
- The Milk-V Megrez board is fast for an embedded board, but still much slower
  than a desktop for Rust builds.

## Fix

Run the build in the background so a terminal disconnect or heartbeat timeout
does not abort it:

```bash
nohup bash -c '
    cd /home/qute-wsl/Program/os-riscv-port/kernel
    . $HOME/.cargo/env
    cargo osdk build --scheme riscv --target-arch riscv64
' > /tmp/aster-build.log 2>&1 &
```

Watch progress:

```bash
tail -f /tmp/aster-build.log
```

## Verification

After some time, `/tmp/aster-build.log` should show crates being compiled and
eventually finish with a successful build message.
