# osdk-test-kernel

This is an [OSDK](https://crates.io/crates/cargo-osdk)-based kernel that solely
runs unit tests. It is shipped with [OSDK](https://crates.io/crates/cargo-osdk)
to provide default unit-test infrastructure for kernel projects based on
[OSTD](https://crates.io/crates/ostd).

This is part of the [Asterinas](https://github.com/asterinas/asterinas)
project.

Kernel tests run in parallel by default. A test that uses shared global state
can opt out with a `#[serial]` attribute; serial tests run one at a time after
all parallel tests finish:

```rust
#[ktest]
#[serial]
fn changes_global_state() {
    // ...
}
```

Every test runs in its own OSTD task. Tasks spawned by a test may request a CPU
affinity by storing a `CpuSet` as their task data. Tasks with other task data
may run on any CPU:

```rust
use ostd::{
    cpu::{CpuId, CpuSet},
    task::TaskOptions,
};

TaskOptions::new(|| {
    // This helper runs on CPU 1.
})
.data(CpuSet::from(CpuId::new(1)))
.spawn()
.unwrap();
```
