# Concurrency and Races

Concurrency code is reviewed with extreme rigor.
Lock ordering, atomic correctness, memory ordering,
and race condition analysis are all demanded explicitly.

### Establish and enforce a consistent lock order (`lock-ordering`) {#lock-ordering}

Acquiring two locks in different orders
from different code paths
is a potential deadlock.
Hierarchical lock order must be established and documented.

```rust
pub(super) fn set_control(
    self: Arc<Self>,
    process: &Process,
) -> Result<()> {
    // Lock order: group of process -> session inner -> job control
    let process_group_mut = process.process_group.lock();
    // ...
}
```

See also:
PR [#2942](https://github.com/asterinas/asterinas/pull/2942).

### Never do I/O or blocking operations while holding a spinlock (`no-io-under-spinlock`) {#no-io-under-spinlock}

Holding a spinlock while performing I/O
or blocking operations is a deadlock hazard.
Use a sleeping mutex or restructure
to drop the lock first.

```rust
// Good — spinlock dropped before I/O
let data = {
    let guard = self.state.lock(); // state: SpinLock<...>
    guard.pending_data.clone()
};
self.device.write(&data)?;

// Bad — I/O while holding spinlock
let guard = self.state.lock(); // state: SpinLock<...>
self.device.write(&guard.pending_data)?;
```

See also:
PR [#925](https://github.com/asterinas/asterinas/pull/925).

### Do not use atomics casually (`careful-atomics`) {#careful-atomics}

When multiple atomic fields
must be updated in concert, use a lock.
Only use atomics when a single value
is genuinely independent.

```rust
// Good — a lock protects correlated fields
struct Stats {
    inner: SpinLock<StatsInner>,
}
struct StatsInner {
    total_bytes: u64,
    total_packets: u64,
}

// Bad — two atomics that must be consistent
// but can be observed in an inconsistent state
struct Stats {
    total_bytes: AtomicU64,
    total_packets: AtomicU64,
}
```

### Critical sections must not be split across lock boundaries (`atomic-critical-sections`) {#atomic-critical-sections}

Operations that must be atomic
(check + conditional action)
must happen under the same lock acquisition.
Moving a comparison outside the critical region
is a correctness bug.

```rust
// Good — check and action under the same lock
let mut inner = self.inner.lock();
if inner.state == State::Ready {
    inner.state = State::Running;
    inner.start();
}

// Bad — TOCTOU race: state can change
// between the check and the action
let is_ready = self.inner.lock().state == State::Ready;
if is_ready {
    self.inner.lock().state = State::Running;
    self.inner.lock().start();
}
```

See also:
PR [#2277](https://github.com/asterinas/asterinas/pull/2277).
