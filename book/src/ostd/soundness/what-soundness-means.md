# What Soundness Means for an OS Framework

## The Soundness Goal

The soundness of OSTD provides the following strong guarantee:

> **No sequence of calls to OSTD's safe public API —
> combined with any behavior from user-space programs and peripheral devices —
> from a client in safe Rust can trigger undefined behaviors (UBs).**

Note that soundness is distinct from correctness.
A sound OSTD may still contain logic bugs
(e.g., a scheduler that starves tasks, or an allocator that wastes memory).
Soundness means these bugs cannot escalate
into memory corruption, data races, type confusion, or any other form of UB.
The OS may crash, deadlock, or behave incorrectly —
but it will never silently corrupt memory.

## Three Levels of Undefined Behavior

An OS kernel faces undefined behavior (UB) threats that ordinary Rust programs do not.
We classify them into three levels:

**Language-level UB.**
These are the UB categories defined by the [Rust Reference](https://doc.rust-lang.org/reference/behavior-considered-undefined.html):
data races, use-after-free, dangling pointers, null pointer dereference,
type confusion, aliasing violations (breaking the "shared XOR mutable" rule),
and reading uninitialized memory.
Rust's safe subset prevents all of these at compile time.
OSTD must ensure that its `unsafe` code does not reintroduce them.

**Environment-level UB.**
Programming languages make implicit assumptions about their execution environment:
the stack is valid, the heap is not corrupted,
and the code being executed has not been overwritten.
In a kernel, these assumptions must be actively maintained.
A stack overflow that jumps over a guard page,
a heap corruption from a faulty allocator,
or a code page overwritten by a rogue DMA transfer
can all silently violate these assumptions.

**Architecture-level UB.**
OS kernels directly manipulate hardware state:
CPU control registers, page tables, interrupt descriptor tables, IOMMU configuration.
Incorrect manipulation can corrupt the execution environment
in ways invisible to the Rust compiler.
For example:
misconfiguring a page table can cause the CPU
to write kernel data to the wrong physical address;
allowing a device to DMA into a page table page can corrupt address translation;
improperly saving/restoring CPU registers across context switches
can corrupt task state.

OSTD must defend against all three levels simultaneously.

> [!NOTE]
> To be concrete, our soundness analysis targets the x86-64 CPU architecture,
> but our arguments can be generalized to other CPU architectures.

## The Trust Model

The following components are trusted — their correctness is assumed:

- **The Rust compiler and core libraries** (`rustc`, `core`, `alloc`).
  We rely on the compiler to correctly enforce Rust's safety guarantees for safe code
  and to correctly compile `unsafe` code according to the language specification.

- **OSTD itself**.
  This is the subject of this document:
  we argue that OSTD's `unsafe` code is sound.

- **The bootloader and firmware**.
  We trust the bootloader to load the kernel correctly,
  provide accurate memory maps, and configure the initial page table.
  We trust firmware (ACPI tables, device trees) to accurately describe the hardware.

- **Core hardware**:
  the CPU (correct instruction execution, correct privilege level enforcement, correct page table walks),
  memory controller (correct physical memory access),
  and IOMMU (correct DMA remapping and interrupt remapping when enabled).

The following components are **not** trusted —
OSTD must remain sound regardless of their behavior:

- **All OS services in `kernel/`**.
  These are written in safe Rust.
  They may contain logic bugs
  (incorrect syscall implementations, file system corruption, network protocol violations)
  but cannot cause UB.
  This includes all device drivers, file systems, networking stacks,
  process management, and IPC.

- **Injected policies**:
  task schedulers, frame allocators, and slab allocators.
  These are registered by OS services via safe traits.
  They may make arbitrarily bad decisions
  (schedule the same task twice, return an already-allocated frame, return a misaligned slab slot)
  but cannot cause UB.
  OSTD's mechanisms enforce soundness independently of policy correctness
  ([Safe Policy Injection](safe-policy-injection.md)).

- **User-space programs**.
  They may execute arbitrary instructions,
  make arbitrary system calls with arbitrary arguments,
  and attempt to corrupt kernel state through any user-accessible mechanism.
  OSTD must ensure that user-space behavior cannot cause kernel UB.

- **Peripheral devices** (NICs, GPUs, disks, USB devices).
  They may issue arbitrary DMA transfers to any physical address,
  generate arbitrary interrupts,
  and return arbitrary data in response to MMIO/PIO reads.
  OSTD must ensure that peripheral behavior cannot corrupt kernel memory safety.
  This is enforced by the IOMMU
  ([Safe Kernel-Peripheral Interactions](safe-kernel-peripheral-interactions.md))
  and the typed/untyped memory model
  ([Safe Physical Memory Management](safe-memory-management.md)).
