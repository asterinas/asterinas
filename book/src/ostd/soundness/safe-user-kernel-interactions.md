# Safe User-Kernel Interactions

OSTD provides three API categories for user-kernel interactions:
entering/exiting user space ([`UserMode`](https://asterinas.github.io/api-docs/0.17.1/ostd/user/struct.UserMode.html), [`UserContext`](https://asterinas.github.io/api-docs/0.17.1/ostd/arch/cpu/context/struct.UserContext.html)),
managing user address spaces ([`VmSpace`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.VmSpace.html)),
and accessing user memory ([`VmReader<Fallible>`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/io/struct.VmReader.html), [`VmWriter<Fallible>`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/io/struct.VmWriter.html)).

## UserMode and UserContext

`UserContext` stores the user-space register state
(general-purpose registers, RFLAGS, FS/GS base, and exception info).
Clients read and write this state between user-space entries —
for instance, to set up syscall return values
or inspect exception details.

`UserMode` owns a `UserContext` and provides
the safe enter/exit loop for user space.
It is `!Send`:
once created on a task,
it cannot be sent to another,
preventing cross-task confusion of register state.
[`UserMode::context()`](https://asterinas.github.io/api-docs/0.17.1/ostd/user/struct.UserMode.html#method.context)
and [`UserMode::context_mut()`](https://asterinas.github.io/api-docs/0.17.1/ostd/user/struct.UserMode.html#method.context_mut)
give access to the underlying `UserContext`
for register inspection and modification between entries.

Clients may set arbitrary values in `UserContext`,
but the sanitization inside
[`UserMode::execute(has_kernel_event)`](https://asterinas.github.io/api-docs/0.17.1/ostd/user/struct.UserMode.html#method.execute)
ensures kernel safety invariants
are enforced every time before entering user space:

1. **RFLAGS sanitization**:
   IF (Interrupt Flag) and ID are forced on.
   IOPL is stripped.
   This prevents user space from disabling interrupts
   or gaining I/O port access through RFLAGS manipulation.

2. **IRQ disabling before transition**:
   Local IRQs are disabled before calling the assembly `syscall_return` routine.
   This is critical:
   the `swapgs` instruction
   (which switches between kernel and user GS base)
   must execute atomically with the actual privilege level transition
   (`sysret` or `iret`).
   If an interrupt occurred between `swapgs` and `sysret`,
   the interrupt handler would see the user GS base instead of the kernel GS base,
   corrupting CPU-local storage access.

3. **FS base**:
   The user's `fsbase` is stored in `UserContext` and restored via `wrfsbase`.
   The kernel uses GS (not FS) for CPU-local storage,
   so a user-controlled FS base cannot affect kernel state.

The above safety measures are key to achieving the following invariant:

> **Safety Invariant:** User space cannot manipulate kernel-mode CPU state.

## VmSpace

`VmSpace` manages a user-space page table.
To modify page table mappings,
clients obtain a [`CursorMut`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.CursorMut.html)
via [`VmSpace::cursor_mut(guard, &range)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.VmSpace.html#method.cursor_mut),
which gives exclusive access to a virtual address range within the page table.

The key safety invariant of `VmSpace` is:

> **Safety Invariant.** Only untyped frames and I/O memory can be mapped into user space.

`CursorMut` exposes exactly two methods for creating mappings,
and neither accepts typed frames:

- [`CursorMut::map(frame, prop)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.CursorMut.html#method.map)
  accepts a `UFrame` — an untyped frame.
  There is no code path that converts a typed `Frame<M>` to `UFrame`
  unless `M: AnyUFrameMeta`.
- [`CursorMut::map_iomem(io_mem, prop, len, offset)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.CursorMut.html#method.map_iomem)
  accepts an `IoMem` (defaulting to `IoMem<Insensitive>`) —
  peripheral MMIO memory that does not host Rust objects.

**TLB coherence**:
When frames are unmapped from user space,
they are wrapped in `RcuDrop` and attached to the [`TlbFlusher`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/tlb.rs#L28),
which tracks which CPUs have cached the translation.

> **Safety Invariant.** The frames are not freed until TLB flush IPIs have been sent to all relevant CPUs and acknowledged.

This prevents use-after-free via stale TLB entries.

## Accessing User Memory from the Kernel

The kernel frequently needs to read/write user-space memory
(e.g., for `read()`/`write()` syscalls).
Because user space can modify its own pages at any time,
creating a Rust reference (`&T` or `&mut T`) to user memory
would violate Rust's aliasing rules —
the same problem that motivates the `VmReader`/`VmWriter` interface
for [untyped frames](safe-memory-management.md#the-solution-typed-and-untyped-frames).

> **Safety Invariant.** The kernel never creates Rust references to user memory.

[`VmSpace::reader(vaddr, len)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.VmSpace.html#method.reader)
and [`VmSpace::writer(vaddr, len)`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.VmSpace.html#method.writer)
are the only safe APIs for user memory access.
They return `VmReader<Fallible>` / `VmWriter<Fallible>` cursors —
the same raw-pointer-based mechanism used for untyped frame access,
but operating on user virtual addresses instead of physical frame contents.
The `Fallible` marker (as opposed to `Infallible` for frame access)
indicates that individual reads and writes may trigger page faults
(e.g., on unmapped or copy-on-write pages);
OSTD handles these faults gracefully
and propagates the error to the caller.

These methods also enforce two runtime checks
before constructing the cursors:

- **Page table identity**:
  The call fails if the `VmSpace`'s page table
  is not the one currently active on this CPU,
  preventing access to another task's address space.
- **Address range**:
  The call fails if the address range extends
  beyond the user-space address limit,
  preventing accidental access to kernel memory through this API.
