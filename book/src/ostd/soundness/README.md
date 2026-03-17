# Soundness Analysis

OSTD is the privileged OS framework —
the memory-safety Trusted Computing Base (TCB) —
of the Asterinas framekernel OS.
OSTD is **privileged** because it is the **only** crate
in the entire Asterinas codebase that is permitted to use `unsafe` Rust.
Every other crate, including the full kernel (`kernel/`),
enforces `#![deny(unsafe_code)]`.

The privilege boundary in an OSTD-based framekernel
is OSTD's **public safe API surface**:
every `pub` function, type, and trait that OSTD exposes.
This document demonstrates that OSTD is **sound**:
its `unsafe` code does not introduce undefined behavior (UB),
regardless of how the rest of the system behaves.

## Outline

This section presents a systematic argument for the soundness of OSTD.
The argument proceeds as follows.
We first define exactly [what soundness means](what-soundness-means.md)
in an OS context and identify the threat model.
We then explain the systematic principle OSTD uses to decide which resources require protection:
[the sensitivity classification](sensitivity-classification.md),
which partitions CPU, memory, and device resources
into sensitive (kept inside OSTD) and insensitive (safely exposed) categories.
This principle motivates every design decision in the sections that follow.

With the threat model and classification in hand,
we address each attack surface in turn.
The most fundamental is physical memory:
if a client could obtain two mutable views of the same memory,
or write to memory hosting Rust objects,
all other guarantees collapse.
[Safe Physical Memory Management](safe-memory-management.md)
presents the typed/untyped memory model —
the cornerstone of OSTD's soundness —
and argues that the frame lifecycle, reference counting, and metadata system prevent these violations.
Building on this memory foundation,
[Safe User-Kernel Interactions](safe-user-kernel-interactions.md)
shows that user-space programs cannot corrupt kernel memory
(because only untyped frames are mapped into user space)
and that the user-kernel transition preserves CPU state integrity.
[Safe Kernel-Peripheral Interactions](safe-kernel-peripheral-interactions.md)
closes the device boundary:
IOMMU DMA remapping restricts devices to untyped memory,
interrupt remapping prevents interrupt spoofing,
and I/O access control hides sensitive hardware registers.

The remaining sections address threats
that arise from within kernel-mode safe Rust code itself.
[Safe Kernel Logic](safe-kernel-logic.md)
demonstrates that OSTD's synchronization primitives, preemption control, and CPU-local storage
cannot be misused by clients to cause data races or deadlocks that lead to UB.
[Safe Policy Injection](safe-policy-injection.md)
shows that even the large, complex policy components injected into OSTD
(scheduler, frame allocator, slab allocator)
cannot violate soundness regardless of their behavior,
because OSTD validates their outputs independently.

Together, these sections cover all major UB risks of OSTD.
