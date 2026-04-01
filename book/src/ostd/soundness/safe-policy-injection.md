# Safe Policy Injection

OSTD separates *mechanism* (in the TCB) from *policy* (outside).
Three complex policy components —
task scheduler, frame allocator, and slab allocator —
are implemented outside OSTD in safe Rust
and injected via safe trait interfaces.
This keeps the TCB minimal
while allowing sophisticated, evolving implementations.

The key principle is:
**OSTD's mechanisms enforce soundness independently of policy correctness.**
A buggy or adversarial (but safe-Rust) policy
may cause incorrect behavior
(wrong scheduling decisions, suboptimal memory allocation),
but it cannot cause UB.

## Task Scheduler

The scheduler is injected via two safe traits:
[`Scheduler<T>`](https://asterinas.github.io/api-docs/0.17.1/ostd/task/scheduler/trait.Scheduler.html) (global scheduling decisions)
and [`LocalRunQueue<T>`](https://asterinas.github.io/api-docs/0.17.1/ostd/task/scheduler/trait.LocalRunQueue.html) (per-CPU run queue management).
Neither trait contains `unsafe` methods.

**The threat**:
A buggy scheduler might return the same task as the "next task"
on two different CPUs simultaneously,
or return a task that is already running.

**The defense**:
The [`switched_to_cpu: AtomicBool`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/task/mod.rs#L67) flag in each `Task`.
In [`before_switching_to()`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/task/processor.rs#L95),
OSTD performs `compare_exchange(false, true, AcqRel, Relaxed)`.
If the flag is already `true`
(the task is running on another CPU),
the CAS fails and the CPU spins until the flag becomes `false`.

This defense is unconditional —
it does not depend on the scheduler being correct.
Even if the scheduler's [`LocalRunQueue::pick_next()`](https://asterinas.github.io/api-docs/0.17.1/ostd/task/scheduler/trait.LocalRunQueue.html#method.pick_next) returns a task
that is actively running on another CPU,
the CAS loop prevents the second CPU from entering the task's context.
The first CPU will eventually switch away from the task
(setting `switched_to_cpu = false` in [`after_switching_to`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/task/processor.rs#L119)),
at which point the second CPU can proceed.

> **Safety Invariant.** A task never executes on two CPUs simultaneously, regardless of scheduler behavior.

Running a task on two CPUs would mean
two CPUs share a single kernel stack —
a catastrophic memory safety violation.
The `switched_to_cpu` CAS prevents this unconditionally.

## Frame Allocator

The frame allocator is injected via the [`GlobalFrameAllocator`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/frame/allocator/trait.GlobalFrameAllocator.html) trait (safe).
The allocator decides which physical addresses to return
for allocation requests.

**The threat**:
A buggy allocator might return an address that is already in use
(double-allocation),
an address outside physical memory,
or an address in a reserved region.

**The defense**:
[`Frame::from_unused(paddr, metadata)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/frame/struct.Frame.html#method.from_unused) enforces the invariant
by checking the frame metadata:

> **Safety Invariant.** The allocator cannot trick OSTD into creating two `Frame` handles to the same physical page.

The metadata system acts as a second line of defense behind the allocator.
Even if the allocator returns a wrong address,
`from_unused` will detect the error
(frame already in use, or out of range) and reject it.
The atomic CAS on the reference count prevents double-allocation.

## Slab Allocator

The slab allocator is injected via the [`GlobalHeapAllocator`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/heap/trait.GlobalHeapAllocator.html) trait (safe).

**The threat**:
A buggy slab allocator might return a slot that is already allocated
(double-allocation),
a slot with wrong size/alignment,
or a slot from a freed slab.

**The defenses**:
OSTD controls key slab allocator building blocks
such as [`Slab`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/heap/type.Slab.html) and [`HeapSlot`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/heap/struct.HeapSlot.html)
to prevent an injected slab allocator
from tricking OSTD into using invalid memory slots.

> **Safety Invariant.** A `Slab` cannot be freed while any of its slots are still allocated.

[`SlabMeta::alloc()`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/heap/struct.SlabMeta.html#method.alloc) increments
and [`Slab::dealloc(slot)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/heap/type.Slab.html#method.dealloc) decrements `nr_allocated`.
The slab panics on drop if `nr_allocated > 0`,
preventing use-after-free of slab memory.
Additionally, `Slab::dealloc(slot)` validates
that the slot's physical address falls within the slab's range,
rejecting slots that do not belong to it.

> **Safety Invariant.** Every heap allocation is backed by a slot whose size and alignment match the requested layout.

[`AllocDispatch`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/heap/mod.rs#L96) (the `GlobalAlloc` shim) validates
every `HeapSlot` returned by the injected allocator:
the slot's size must match the size determined by
[`slot_size_from_layout`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/heap/mod.rs#L76),
it must be at least as large as the requested layout,
and its address must satisfy the layout's alignment requirement.
Mismatches cause abort before the memory is used.

> **Safety Invariant.** Client code cannot forge a `HeapSlot`.

[`HeapSlot::new()`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/heap/slot.rs#L69) is `pub(super) unsafe`,
so only OSTD's heap module can create `HeapSlot` values.
The injected allocator receives `HeapSlot`s
from `SlabMeta::alloc()` or [`HeapSlot::alloc_large(size)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/heap/struct.HeapSlot.html#method.alloc_large)
and must return them through `GlobalHeapAllocator::dealloc()` —
it cannot construct arbitrary slots pointing to sensitive memory.

## Other Injected Callbacks

Beyond the three complex policies above,
OSTD has simpler injection points
for [logging](https://asterinas.github.io/api-docs/0.17.1/ostd/log/logger/fn.inject_logger.html),
[power management](https://asterinas.github.io/api-docs/0.17.1/ostd/power/fn.inject_restart_handler.html),
[scheduling hooks](https://asterinas.github.io/api-docs/0.17.1/ostd/task/fn.inject_pre_schedule_handler.html),
[page fault handling](https://asterinas.github.io/api-docs/0.17.1/ostd/arch/trap/fn.inject_user_page_fault_handler.html),
[SMP boot](https://asterinas.github.io/api-docs/0.17.1/ostd/boot/smp/fn.register_ap_entry.html),
[interrupt bottom halves](https://asterinas.github.io/api-docs/0.17.1/ostd/irq/fn.register_bottom_half_handler_l1.html),
and [timer callbacks](https://asterinas.github.io/api-docs/0.17.1/ostd/timer/fn.register_callback_on_cpu.html).

These callbacks are fundamentally different
from the scheduler and allocators:
they do not return information
that OSTD must validate for correctness.
A buggy scheduler can return the wrong task;
a buggy allocator can return the wrong address.
These callbacks simply *do something* when invoked —
log a message, handle a fault, process deferred work.

> **Safety Invariant.** No injected callback can compromise memory safety.

Two properties guarantee this.
First, every callback is a safe function pointer or trait object,
and the kernel enforces `#![deny(unsafe_code)]`,
so injected code is constrained to safe Rust.
Second, OSTD's execution context
at each invocation point
prevents the most dangerous side effects.
Schedule hooks and bottom-half handlers
run with IRQs or preemption disabled,
which prevents sleeping and limits reentrancy.
Timer callbacks run in IRQ-disabled interrupt context
with a `RefCell` borrow guard
that detects reentrant registration.
Power handlers are divergent (`-> !`)
with a machine-halt fallback.

The one notable exception is the logger:
it can be called from virtually any context —
normal tasks, interrupt handlers, panic handlers,
and while OSTD holds internal locks.
A logger that allocates, sleeps, or holds its own lock
risks deadlock or cascading panics.
The doc comment on `inject_logger`
warns that the implementation must be
simple, non-sleeping, and heapless,
but this is advisory rather than enforced.
Even so, the consequences are liveness failures
(hangs, aborts) — not memory safety violations.
