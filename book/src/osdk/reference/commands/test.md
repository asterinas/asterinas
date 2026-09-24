# cargo osdk test

`cargo osdk test` is used to
execute kernel mode unit test by starting QEMU.
The usage is as follows:

```bash
cargo osdk test [TESTNAME] [OPTIONS]
```

## Arguments

`TESTNAME`:
Only run tests containing this string in their names

## Options

The options are the same as those of `cargo osdk build`.
Refer to the [documentation](build.md) of `cargo osdk build`
for more details.

## Parallel and serial tests

OSDK runs kernel-mode unit tests in parallel when the test VM has multiple
CPUs. Each test runs in its own OSTD task. Tests that share global state can be
marked `#[serial]`; they run one at a time after all parallel tests finish.

```rust
#[ktest]
#[serial]
fn changes_global_state() {
    // ...
}
```

A helper task can be pinned to one or more CPUs by using a `CpuSet` as its task
data:

```rust
TaskOptions::new(helper)
    .data(CpuSet::from(CpuId::new(1)))
    .spawn()
    .unwrap();
```

## Examples
- Execute tests that include *foo* in their names
using QEMU with 3GB of memory

```bash
cargo osdk test foo --qemu-args="-m 3G"
```
