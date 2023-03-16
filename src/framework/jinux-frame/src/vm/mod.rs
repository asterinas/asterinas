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
    page_table::{translate_not_offset_virtual_address, PageTable},
};

use alloc::vec::Vec;
use limine::LimineMemmapRequest;
use log::debug;
use spin::Once;

pub const fn phys_to_virt(pa: usize) -> usize {
    pa + PHYS_OFFSET
}

pub const fn virt_to_phys(va: usize) -> usize {
    va - PHYS_OFFSET
}

pub const fn align_down(p: usize) -> usize {
    p & !(PAGE_SIZE - 1)
}

pub const fn align_up(p: usize) -> usize {
    (p + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

pub const fn page_offset(p: usize) -> usize {
    p & (PAGE_SIZE - 1)
}

pub const fn is_aligned(p: usize) -> bool {
    page_offset(p) == 0
}

/// Only available inside jinux-frame
pub(crate) static MEMORY_REGIONS: Once<Vec<&limine::LimineMemmapEntry>> = Once::new();
static MEMMAP_REQUEST: LimineMemmapRequest = LimineMemmapRequest::new(0);

pub(crate) fn init() {
    heap_allocator::init();
    let mut memory_regions = Vec::new();
    let response = MEMMAP_REQUEST
        .get_response()
        .get()
        .expect("Not found memory region information");
    for i in response.memmap() {
        debug!("Found memory region:{:x?}", **i);
        memory_regions.push(&**i);
    }

    frame_allocator::init(&memory_regions);
    page_table::init();

    MEMORY_REGIONS.call_once(|| memory_regions);
}
