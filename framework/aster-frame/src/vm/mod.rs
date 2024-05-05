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
mod io;
pub(crate) mod kspace;
mod offset;
mod options;
pub(crate) mod page_prop;
pub(crate) mod page_table;
mod space;

use alloc::{borrow::ToOwned, vec::Vec};
use core::{fmt::Debug, ops::Range};

use spin::Once;

pub use self::{
    dma::{Daddr, DmaCoherent, DmaDirection, DmaStream, DmaStreamSlice, HasDaddr},
    frame::{VmFrame, VmFrameVec, VmFrameVecIter, VmReader, VmSegment, VmWriter},
    io::VmIo,
    options::VmAllocOptions,
    page_prop::{CachePolicy, PageFlags, PageProperty},
    space::{VmMapOptions, VmSpace},
};
pub(crate) use self::{
    kspace::paddr_to_vaddr, page_prop::PrivilegedPageFlags, page_table::PageTable,
};
use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};

/// The size of a [`VmFrame`].
pub const PAGE_SIZE: usize = 0x1000;

/// A minimal set of constants that determines the paging system.
/// This provides an abstraction over most paging modes in common architectures.
pub(crate) trait PagingConstsTrait: Debug + 'static {
    /// The smallest page size.
    /// This is also the page size at level 1 page tables.
    const BASE_PAGE_SIZE: usize;

    /// The number of levels in the page table.
    /// The numbering of levels goes from deepest node to the root node. For example,
    /// the level 1 to 5 on AMD64 corresponds to Page Tables, Page Directory Tables,
    /// Page Directory Pointer Tables, Page-Map Level-4 Table, and Page-Map Level-5
    /// Table, respectively.
    const NR_LEVELS: usize;

    /// The highest level that a PTE can be directly used to translate a VA.
    /// This affects the the largest page size supported by the page table.
    const HIGHEST_TRANSLATION_LEVEL: usize;

    /// The size of a PTE.
    const PTE_SIZE: usize;
}

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

/// The kernel address space.
/// There are the high canonical addresses defined in most 48-bit width
/// architectures.
pub(crate) const KERNEL_VADDR_RANGE: Range<Vaddr> = 0xffff_8000_0000_0000..0xffff_ffff_ffff_0000;

/// Get physical address trait
pub trait HasPaddr {
    fn paddr(&self) -> Paddr;
}

pub const fn is_page_aligned(p: usize) -> bool {
    (p & (PAGE_SIZE - 1)) == 0
}

/// Only available inside aster-frame
pub(crate) static MEMORY_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub static FRAMEBUFFER_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub(crate) fn init() {
    let memory_regions = crate::boot::memory_regions().to_owned();
    frame_allocator::init(&memory_regions);
    kspace::init_kernel_page_table();
    dma::init();

    let mut framebuffer_regions = Vec::new();
    for i in memory_regions.iter() {
        if i.typ() == MemoryRegionType::Framebuffer {
            framebuffer_regions.push(*i);
        }
    }
    FRAMEBUFFER_REGIONS.call_once(|| framebuffer_regions);

    MEMORY_REGIONS.call_once(|| memory_regions);
}
