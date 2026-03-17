# Safe Kernel Logic

OSTD provides preemption control, synchronization primitives,
and CPU-local storage for OS services.
These must be sound even when used
by potentially buggy (but safe-Rust) clients.

## Preemption and Atomic Mode

In a preemptive kernel,
a running task can be interrupted at almost any point
and replaced by another task on the same CPU.
Certain operations —
such as holding a spinlock or executing an interrupt handler —
require that preemption be temporarily disabled.
OSTD calls this state **atomic mode**:
a CPU is in atomic mode
when preemption is disabled or local IRQs are disabled.

Sleeping while in atomic mode is a classic kernel bug.
If a spinlock holder sleeps,
the lock remains held while the CPU runs another task;
if that task tries to acquire the same lock, a deadlock results.
OSTD prevents this unconditionally:

> **Safety Invariant.** A task cannot sleep while in atomic mode.

OSTD tracks atomic mode using guard types.
[`DisabledPreemptGuard`](https://asterinas.github.io/api-docs/0.17.1/ostd/task/struct.DisabledPreemptGuard.html)
disables preemption for its lifetime,
and [`DisabledLocalIrqGuard`](https://asterinas.github.io/api-docs/0.17.1/ostd/irq/struct.DisabledLocalIrqGuard.html)
disables local IRQs
(which also implies preemption is disabled).
Both guards are `!Send` —
they cannot be moved to another CPU,
which would corrupt the CPU-local preemption state.

Every context switch and every sleeping-lock wait path
checks that the current CPU is not in atomic mode,
panicking if the check fails.
This enforcement is comprehensive:
a spinlock's guard puts the CPU in atomic mode,
so any attempt to sleep while holding a spinlock is caught.

## Synchronization Primitives

OSTD provides six synchronization primitives:
[`SpinLock`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.SpinLock.html), [`Mutex`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.Mutex.html), [`RwLock`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.RwLock.html), [`RwMutex`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.RwMutex.html), [`Rcu`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.Rcu.html), and [`WaitQueue`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.WaitQueue.html).
Their soundness rests on two design principles:

**Principle 1: Guards are `!Send`.**
Every lock guard type
(`SpinLockGuard`, `MutexGuard`, `RwLockReadGuard`, etc.) is `!Send`.
This prevents a client from acquiring a lock on one CPU
and releasing it on another —
which would corrupt the CPU-local preemption state
(since the embedded preemption or IRQ guard
is tied to the acquiring CPU).

**Principle 2: Atomic mode is entered before lock acquisition.**
[`SpinLock::lock()`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.SpinLock.html#method.lock)
disables preemption (or IRQs) *before* entering the CAS loop.
If the order were reversed (acquire lock, then disable preemption),
there would be a window where the lock is held
but preemption is still enabled —
an interrupt could preempt the task,
and if the interrupt handler tries to acquire the same lock,
a deadlock results.

The kind of guard —
preemption-only ([`PreemptDisabled`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.PreemptDisabled.html))
or IRQ-disabled ([`LocalIrqDisabled`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.LocalIrqDisabled.html)) —
is chosen at lock declaration time
and statically enforced by the type system,
ensuring consistent usage throughout the lock's lifetime.

### RCU (Read-Copy-Update)

[`Rcu<P>`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.Rcu.html) provides lock-free read access with deferred reclamation:

- **Read side**:
  [`Rcu::read()`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.Rcu.html#method.read) disables preemption
  and loads the pointer with `Acquire` ordering.
  The read-side critical section is non-preemptible.
- **Write side**:
  [`Rcu::update(new_ptr)`](https://asterinas.github.io/api-docs/0.17.1/ostd/sync/struct.Rcu.html#method.update) atomically swaps the pointer
  and enqueues the old value for deferred drop.
- **Grace period**:
  OSTD detects when all CPUs have left
  their read-side critical sections
  by observing context switches.
  A CPU that has performed a context switch
  must have passed through preemption-enabled state
  (since the context switch path enforces that),
  and therefore cannot still be inside a read-side critical section.
  Once all CPUs have been observed in this state,
  deferred drops are executed safely.

> **Safety Invariant.** No safe API allows a client to violate lock invariants.

The guard type system
(`!Send`, atomic mode tracking, statically-chosen guard kinds)
ensures that locks are used correctly regardless of client behavior.

## CPU-Local Storage

OSTD provides two CPU-local storage mechanisms:

**`cpu_local!` (static variables)**:
Access requires a `DisabledLocalIrqGuard`
via [`CpuLocal::get_with(&irq_guard)`](https://asterinas.github.io/api-docs/0.17.1/ostd/cpu/local/struct.CpuLocal.html#method.get_with).
This ensures two properties:
- The task is pinned to the current CPU
  (IRQs disabled implies no preemption and no migration).
- No interrupt handler can observe a partially-modified state.

For `Sync` types,
[`CpuLocal::get_on_cpu(target_cpu)`](https://asterinas.github.io/api-docs/0.17.1/ostd/cpu/local/struct.CpuLocal.html#method.get_on_cpu) allows remote access without a guard
(since `Sync` types are safe to share across threads).

**`cpu_local_cell!` (cell variables)**:
These provide inner mutability through operations
that are atomic with respect to interrupts on the same CPU.
OSTD uses this mechanism internally
for preemption state and current-task tracking,
which must be modified without risk of interrupt interference.

> **Safety Invariant.** No safe API allows a client to access another CPU's local storage without proper synchronization.

Static CPU-local variables require an IRQ guard to access,
ensuring CPU pinning.
Cell variables use interrupt-safe operations by construction.
