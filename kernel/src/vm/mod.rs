// SPDX-License-Identifier: MPL-2.0

//! Virtual memory (VM).
//!
//! There are two primary VM abstractions:
//!  * The VMAR (used to be Virtual Memory Address Region, now an orphan
//!    initialism) represents the entire virtual address space of a process;
//!  * The VMO (Virtual Memory Object) is a set of logically contiguous memory
//!    frames that can be mapped into one virtual address range. Frames in a
//!    VMO can be non-contiguous in physical memory.

use osdk_frame_allocator::FrameAllocator;
use osdk_heap_allocator::{HeapAllocator, type_from_layout};

pub mod perms;
pub mod vmar;
pub mod vmo;

#[ostd::global_frame_allocator]
static FRAME_ALLOCATOR: FrameAllocator = FrameAllocator;

#[ostd::global_heap_allocator]
static HEAP_ALLOCATOR: HeapAllocator = HeapAllocator;

#[ostd::global_heap_allocator_slot_map]
const fn slot_type_from_layout(layout: core::alloc::Layout) -> Option<ostd::mm::heap::SlotInfo> {
    type_from_layout(layout)
}

/// Total physical memory in the entire system in bytes.
pub fn mem_total() -> usize {
    use ostd::boot::{boot_info, memory_region::MemoryRegionType};

    let regions = &boot_info().memory_regions;
    regions
        .iter()
        .filter(|region| region.typ() == MemoryRegionType::Usable)
        .map(|region| region.len())
        .sum::<usize>()
}
