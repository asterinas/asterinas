//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

mod frame;
mod frame_allocator;
mod heap_allocator;
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

use alloc::vec::Vec;
use limine::{LimineMemmapRequest, LimineMemoryMapEntryType};
use log::debug;
use spin::Once;

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
pub(crate) static MEMORY_REGIONS: Once<Vec<&limine::LimineMemmapEntry>> = Once::new();
static MEMMAP_REQUEST: LimineMemmapRequest = LimineMemmapRequest::new(0);

pub static FRAMEBUFFER_REGIONS: Once<Vec<&limine::LimineMemmapEntry>> = Once::new();

pub(crate) fn init() {
    heap_allocator::init();
    let mut memory_regions = Vec::new();
    let mut framebuffer_regions = Vec::new();
    let response = MEMMAP_REQUEST
        .get_response()
        .get()
        .expect("Not found memory region information");
    for i in response.memmap() {
        debug!("Found memory region:{:x?}", **i);
        memory_regions.push(&**i);
        if i.typ == LimineMemoryMapEntryType::Framebuffer {
            framebuffer_regions.push(&**i);
        }
    }

    frame_allocator::init(&memory_regions);
    page_table::init();

    MEMORY_REGIONS.call_once(|| memory_regions);
    FRAMEBUFFER_REGIONS.call_once(|| framebuffer_regions);
}
