//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

mod frame;
mod frame_allocator;
pub(crate) mod heap_allocator;
mod io;
mod memory_set;
mod offset;
pub(crate) mod page_table;
mod space;

use crate::config::{PAGE_SIZE, PHYS_OFFSET};

pub use self::frame::{VmAllocOptions, VmFrame, VmFrameVec, VmFrameVecIter};
pub use self::io::VmIo;
pub use self::space::{VmMapOptions, VmPerm, VmSpace};

pub use self::{
    memory_set::{MapArea, MemorySet},
    page_table::PageTable,
};

use alloc::borrow::ToOwned;
use alloc::vec::Vec;
use spin::Once;

use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};

/// Convert physical address to virtual address using offset, only available inside jinux-frame
pub(crate) fn paddr_to_vaddr(pa: usize) -> usize {
    pa + PHYS_OFFSET
}

pub fn vaddr_to_paddr(va: Vaddr) -> Option<Paddr> {
    if va >= crate::config::PHYS_OFFSET && va <= crate::config::KERNEL_OFFSET {
        // can use offset to get the physical address
        Some(va - PHYS_OFFSET)
    } else {
        page_table::vaddr_to_paddr(va)
    }
}

pub const fn is_page_aligned(p: usize) -> bool {
    (p & (PAGE_SIZE - 1)) == 0
}

/// Only available inside jinux-frame
pub(crate) static MEMORY_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub static FRAMEBUFFER_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub(crate) fn init() {
    let memory_regions = crate::boot::memory_regions().to_owned();
    frame_allocator::init(&memory_regions);

    let mut framebuffer_regions = Vec::new();
    for i in memory_regions.iter() {
        if i.typ() == MemoryRegionType::Framebuffer {
            framebuffer_regions.push(i.clone());
        }
    }
    FRAMEBUFFER_REGIONS.call_once(|| framebuffer_regions);

    MEMORY_REGIONS.call_once(|| memory_regions);
}
