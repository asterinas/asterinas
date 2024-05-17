// SPDX-License-Identifier: MPL-2.0

//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

pub(crate) mod dma;
mod frame;
mod frame_allocator;
pub(crate) mod heap_allocator;
pub(crate) mod kvmar_allocator;
pub(crate) mod kvmar;
mod io;
mod memory_set;
mod offset;
mod options;
pub(crate) mod page_table;
mod space;

use alloc::{borrow::ToOwned, vec::Vec};

use spin::Once;

pub use self::{
    dma::{DmaCoherent, DmaDirection, DmaStream, HasDaddr},
    frame::{VmFrame, VmFrameVec, VmFrameVecIter, VmReader, VmSegment, VmWriter},
    io::VmIo,
    memory_set::{MapArea, MemorySet},
    options::VmAllocOptions,
    page_table::PageTable,
    space::{VmMapOptions, VmPerm, VmSpace},
};
use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};

pub const PAGE_SIZE: usize = 0x1000;

/// The maximum virtual address of user space (non inclusive).
///
/// Typicall 64-bit systems have at least 48-bit virtual address space.
/// A typical way to reserve half of the address space for the kernel is
/// to use the highest 48-bit virtual address space.
///
/// Also, the top page is not regarded as usable since it's a workaround
/// for some x86_64 CPUs' bugs. See
/// <https://github.com/torvalds/linux/blob/480e035fc4c714fb5536e64ab9db04fedc89e910/arch/x86/include/asm/page_64.h#L68-L78>
/// for the rationale.
pub const MAX_USERSPACE_VADDR: Vaddr = 0x0000_8000_0000_0000 - PAGE_SIZE;

/// Start of the kernel address space.
///
/// This is the _lowest_ address of the x86-64's _high_ canonical addresses.
///
/// This is also the base address of the direct mapping of all physical
/// memory in the kernel address space.
pub(crate) const PHYS_MEM_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000;

pub const KERNEL_STACK_BASE_VADDR: Vaddr = 0xffff_ffff_0000_0000;

pub const KERNEL_STACK_END_VADDR: Vaddr = 0xffff_ffff_8000_0000;

/// The kernel code is linear mapped to this address.
///
/// FIXME: This offset should be randomly chosen by the loader or the
/// boot compatibility layer. But we disabled it because the framework
/// doesn't support relocatable kernel yet.
pub fn kernel_loaded_offset() -> usize {
    0xffff_ffff_8000_0000
}

/// Get physical address trait
pub trait HasPaddr {
    fn paddr(&self) -> Paddr;
}

pub fn vaddr_to_paddr(va: Vaddr) -> Option<Paddr> {
    if (PHYS_MEM_BASE_VADDR..=kernel_loaded_offset()).contains(&va) {
        // can use offset to get the physical address
        Some(va - PHYS_MEM_BASE_VADDR)
    } else {
        page_table::vaddr_to_paddr(va)
    }
}

pub const fn is_page_aligned(p: usize) -> bool {
    (p & (PAGE_SIZE - 1)) == 0
}

/// Convert physical address to virtual address using offset, only available inside aster-frame
pub(crate) fn paddr_to_vaddr(pa: usize) -> usize {
    pa + PHYS_MEM_BASE_VADDR
}

/// Only available inside aster-frame
pub(crate) static MEMORY_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub static FRAMEBUFFER_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub(crate) fn init() {
    let memory_regions = crate::boot::memory_regions().to_owned();
    frame_allocator::init(&memory_regions);
    page_table::init();
    dma::init();

    kvmar_allocator::init(KERNEL_STACK_BASE_VADDR, KERNEL_STACK_END_VADDR);

    let mut framebuffer_regions = Vec::new();
    for i in memory_regions.iter() {
        if i.typ() == MemoryRegionType::Framebuffer {
            framebuffer_regions.push(*i);
        }
    }
    FRAMEBUFFER_REGIONS.call_once(|| framebuffer_regions);

    MEMORY_REGIONS.call_once(|| memory_regions);
}
