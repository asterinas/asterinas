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
mod memory_set;
mod offset;
mod options;
pub(crate) mod page_table;
mod space;

use crate::config::{KERNEL_OFFSET, PAGE_SIZE, PHYS_OFFSET};

pub use self::dma::{DmaCoherent, DmaDirection, DmaStream, HasDaddr};
pub use self::frame::{VmFrame, VmFrameVec, VmFrameVecIter, VmReader, VmSegment, VmWriter};
pub use self::io::VmIo;
pub use self::options::VmAllocOptions;
pub use self::space::{VmMapOptions, VmPerm, VmSpace};

pub use self::{
    memory_set::{MapArea, MemorySet},
    page_table::PageTable,
};

use alloc::borrow::ToOwned;
use alloc::vec::Vec;
use spin::Once;

use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};

/// Get physical address trait
pub trait HasPaddr {
    fn paddr(&self) -> Paddr;
}

pub fn vaddr_to_paddr(va: Vaddr) -> Option<Paddr> {
    if (PHYS_OFFSET..=KERNEL_OFFSET).contains(&va) {
        // can use offset to get the physical address
        Some(va - PHYS_OFFSET)
    } else {
        page_table::vaddr_to_paddr(va)
    }
}

pub const fn is_page_aligned(p: usize) -> bool {
    (p & (PAGE_SIZE - 1)) == 0
}

/// Convert physical address to virtual address using offset, only available inside aster-frame
pub(crate) fn paddr_to_vaddr(pa: usize) -> usize {
    pa + PHYS_OFFSET
}

/// Only available inside aster-frame
pub(crate) static MEMORY_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub static FRAMEBUFFER_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub(crate) fn init() {
    let memory_regions = crate::boot::memory_regions().to_owned();
    frame_allocator::init(&memory_regions);
    page_table::init();
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
