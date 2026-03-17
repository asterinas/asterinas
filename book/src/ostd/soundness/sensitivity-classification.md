# The Sensitivity Classification Principle

At the heart of the framekernel architecture
is a systematic classification of OS resources.
An OS manages three fundamental classes of resources —
CPU, memory, and devices —
each subdivided into
**sensitive** (can compromise kernel memory safety if misused)
and **insensitive** (cannot compromise kernel memory safety even if misused).
To ensure the soundness and minimality of TCB,
OSTD adopts the following design principle:

> Keep sensitive resources inside the framework (for soundness)
> and move insensitive resources outside (for minimality).

## CPU Resources

| Resource | Classification | Rationale | OSTD Handling |
|----------|---------------|-----------|---------------|
| Kernel-mode registers (CR0–CR4, MSRs, GDT/IDT/TSS pointers, kernel GS base) | **Sensitive** | Can corrupt execution environment | Set once at boot; never exposed to clients |
| Kernel-mode traps (exception/interrupt handlers) | **Sensitive** | Can hijack control flow | IDT configured at boot; handlers are internal |
| User-mode registers (GP registers, user RFLAGS subset, FS base) | Insensitive | Cannot directly affect kernel state | Exposed via [`UserContext`](https://asterinas.github.io/api-docs/0.17.1/ostd/arch/cpu/context/struct.UserContext.html) with sanitization |
| User-mode traps | Insensitive | Routed through kernel trap handler | Dispatched by OSTD; clients handle via callbacks |

`UserContext` sanitizes user-visible registers:
it forces the IF (Interrupt Flag) and ID flags in RFLAGS and strips IOPL,
preventing user space from disabling interrupts
or accessing I/O ports directly.
Kernel-mode registers are never represented in any client-visible type.

## Memory Resources

| Resource | Classification | Rationale | OSTD Handling |
|----------|---------------|-----------|---------------|
| Kernel code pages | **Sensitive** | Overwriting code = arbitrary execution | Mapped read-only; typed frames; never exposed |
| Kernel stack pages | **Sensitive** | Stack corruption = control flow hijack | Typed frames; guard pages; never exposed |
| Kernel heap pages | **Sensitive** | Heap corruption = type confusion | Typed frames ([`SlabMeta`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/heap/slab.rs#L31)); never exposed |
| Page table pages | **Sensitive** | PT corruption = arbitrary memory access | Typed frames ([`PageTablePageMeta`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/page_table/node/mod.rs#L265)); managed by OSTD |
| Frame metadata pages | **Sensitive** | Metadata corruption = use-after-free | In dedicated [`FRAME_METADATA_RANGE`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/mm/kspace/mod.rs#L114); never exposed |
| User-space virtual memory | Insensitive | Kernel safety does not depend on it | Manipulated safely via [`VmSpace`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/vm_space/struct.VmSpace.html) |
| Untyped physical memory pages | Insensitive | Do not host Rust objects; accessed only via POD copy | Exposed as [`UFrame`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/frame/untyped/type.UFrame.html) / [`USegment`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/frame/segment/type.USegment.html); used for user pages, DMA buffers |

The sensitive/insensitive distinction maps directly
to the typed/untyped frame distinction
([Safe Physical Memory Management](safe-memory-management.md)).
All sensitive memory is held in typed frames,
which are never exposed to clients, user space, or devices.
All insensitive memory is held in untyped frames,
which can be safely shared.

## Device Resources

| Resource | Classification | Rationale | OSTD Handling |
|----------|---------------|-----------|---------------|
| Local APIC | **Sensitive** | Can reset CPUs, mask interrupts | [`IoMem`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoMem.html)`<Sensitive>`, `pub(crate)` only |
| I/O APIC | **Sensitive** | Can redirect interrupts | `IoMem<Sensitive>`, `pub(crate)` only |
| IOMMU | **Sensitive** | Controls DMA access | `IoMem<Sensitive>`, `pub(crate)` only |
| PIC (8259A) | **Sensitive** | Legacy interrupt controller | Sensitive I/O ports, `pub(crate)` only |
| Peripheral MMIO (NIC, disk, USB, GPU) | Insensitive | Failure confined to the device | `IoMem<Insensitive>` via [`IoMem::acquire(range)`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoMem.html#method.acquire) |
| Peripheral I/O ports | Insensitive | Failure confined to the device | [`IoPort`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoPort.html) via [`IoPort::acquire(port)`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoPort.html#method.acquire) |
| Peripheral interrupts | Insensitive | Routed through OSTD's IRQ framework | [`IrqLine`](https://asterinas.github.io/api-docs/0.17.1/ostd/irq/struct.IrqLine.html) callback registration |
| DMA mappings | Insensitive | Restricted by IOMMU to untyped memory | [`DmaCoherent`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/dma/struct.DmaCoherent.html) / [`DmaStream`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/dma/struct.DmaStream.html) APIs |

Internally, OSTD maintains two I/O resource allocators ([`IoMemAllocator`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/io/io_mem/allocator.rs#L18), [`IoPortAllocator`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/io/io_port/allocator.rs#L17))
that are initialized at boot time by removing all sensitive ranges.
Only the remaining insensitive ranges are available for client allocation.
This makes it impossible for a client
to accidentally or maliciously access a sensitive device.
