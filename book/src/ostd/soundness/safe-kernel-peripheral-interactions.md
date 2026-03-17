# Safe Kernel-Peripheral Interactions

OSTD provides APIs for device drivers to interact with peripheral hardware:
DMA memory, MMIO, PIO, and interrupt handling.
All are designed so that a safe-Rust driver
cannot compromise kernel memory safety.

## DMA Safety

DMA (Direct Memory Access) allows peripheral devices
to read/write physical memory independently of the CPU.
Without protection,
a malicious or buggy device could DMA into kernel code, page tables, or the heap —
catastrophic for memory safety.

OSTD provides two DMA abstractions,
both restricted to untyped memory:

* **[`DmaCoherent`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/dma/struct.DmaCoherent.html)**: Cache-coherent DMA mapping.
  Created from a [`Segment<()>`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/frame/segment/struct.Segment.html) (untyped).
* **[`DmaStream`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/dma/struct.DmaStream.html)**: Streaming DMA mapping.
  Created from a [`USegment`](https://asterinas.github.io/api-docs/0.17.1/ostd/mm/frame/segment/type.USegment.html) (untyped).

**IOMMU enforcement**:
When IOMMU DMA remapping is enabled (the default on x86),
OSTD configures the IOMMU so that
no physical memory is DMA-accessible by default.
DMA mappings are created on-demand via `iommu::map`,
which inserts entries into the IOMMU page table.
Only addresses that have been explicitly mapped
via `DmaCoherent` or `DmaStream` are accessible to devices.
Since these APIs only accept untyped memory,
the IOMMU page table only ever contains mappings to untyped frames.

> **Safety Invariant.** Devices can only DMA to untyped memory.

A peripheral device can only DMA to addresses
present in the IOMMU page table.
The IOMMU page table only maps untyped memory.
Untyped memory does not host Rust objects.
Therefore, even a malicious device performing arbitrary DMA
cannot corrupt any Rust data structure.

Without IOMMU (when hardware lacks it),
OSTD cannot enforce DMA restrictions.
In this case, the threat model explicitly excludes malicious devices —
but the API-level restriction (untyped-only)
still prevents accidental DMA to sensitive memory by well-behaved drivers.

## Interrupt Remapping

Beyond DMA, devices can also send interrupts —
and on x86, interrupt delivery involves writing to a physical address
(the APIC's message signaled interrupt region).
Without protection, a device could forge interrupts to arbitrary vectors,
potentially triggering unexpected trap handlers.

OSTD enables IOMMU interrupt remapping,
which interposes a translation table
between device interrupt requests and CPU interrupt delivery.
The interrupt remapping table is managed exclusively by OSTD —
no client can access or modify it.

> **Safety Invariant.** Devices cannot inject interrupts to arbitrary vectors.

All interrupt delivery is mediated by the remapping table,
which OSTD controls exclusively.

## I/O Memory Access Control

MMIO (Memory-Mapped I/O) allows the CPU to interact with device registers
by reading/writing specific physical addresses.
Some MMIO regions are sensitive (APIC, IOMMU registers)
and must not be accessible to device drivers.

OSTD's [`IoMem`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoMem.html) type has two sensitivity levels,
enforced at the type level:

```rust
pub struct IoMem<S: SecuritySensitivity> { ... }
```

- `IoMem<Sensitive>`:
  The `read_once` and `write_once` methods are `pub(crate) unsafe`.
  Only OSTD-internal system drivers (APIC, IOMMU, I/O APIC)
  can access sensitive I/O memory.
- `IoMem<Insensitive>`:
  Provides safe `reader()` and `writer()` methods
  via the `HasVmReaderWriter` trait.
  Available to device drivers via [`IoMem::acquire(range)`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoMem.html#method.acquire).

The [`IoMemAllocator`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/io/io_mem/allocator.rs#L18) enforces the boundary:
1. At initialization,
   it is populated with all MMIO address ranges
   that do not overlap with RAM (from firmware tables).
2. System device ranges (APIC, IOMMU, etc.) are removed via `remove()`.
3. Only the remaining ranges are available for `acquire()`.

> **Safety Invariant.** Device drivers cannot access sensitive I/O memory through any safe API.

The `IoMemAllocator` has removed all sensitive ranges
before clients can access it.
Even if a driver obtains `IoMem<Insensitive>` for a peripheral's MMIO,
the worst it can do is misconfigure that specific peripheral —
it cannot access kernel-critical hardware.

## I/O Port Access Control (x86)

I/O ports,
represented as [`IoPort`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoPort.html),
are an x86-specific mechanism for device communication.
The same sensitivity distinction applies:

- `IoPort::new(port)` is `pub(crate) unsafe` —
  for OSTD-internal use only.
- [`IoPort::acquire(port)`](https://asterinas.github.io/api-docs/0.17.1/ostd/io/struct.IoPort.html#method.acquire) is the safe public API.
  It requests the port from the [`IoPortAllocator`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/io/io_port/allocator.rs#L17).

The `IoPortAllocator` removes sensitive ports at initialization.
Sensitive ports are declared statically via the [`sensitive_io_port!`](https://github.com/asterinas/asterinas/blob/9ea44ed2b60bc81a5efb18af79e41fc07bf3d523/ostd/src/io/io_port/mod.rs#L168) macro,
which places their addresses in the `.sensitive_io_ports` linker section.
At boot, OSTD reads this section
and removes all listed ports from the allocator.

> **Safety Invariant.** Device drivers cannot access sensitive I/O ports through any safe API.

Same enforcement as I/O memory —
the `IoPortAllocator` removes all sensitive ports
before clients can access it.

## Interrupt Handling

Device interrupts are managed through the [`IrqLine`](https://asterinas.github.io/api-docs/0.17.1/ostd/irq/struct.IrqLine.html) abstraction.
Clients register callbacks
but cannot manipulate the IDT, APIC, or interrupt routing directly.
All interrupt configuration is mediated by OSTD's safe APIs.

> **Safety Invariant.** Clients cannot corrupt interrupt delivery mechanisms.

They have no access to the IDT, APIC registers,
or interrupt remapping table.
