//! memory management.

pub mod address;
mod frame_allocator;
mod heap_allocator;
mod memory_set;
pub(crate) mod page_table;

pub use self::{
    frame_allocator::PhysFrame,
    memory_set::{MapArea, MemorySet},
    page_table::{translate_not_offset_virtual_address, PageTable},
};

use address::PhysAddr;
use address::VirtAddr;
use alloc::vec::Vec;
use limine::LimineMemmapRequest;
use log::debug;
use spin::Once;

bitflags::bitflags! {
  /// Possible flags for a page table entry.
  pub struct PTFlags: usize {
    /// Specifies whether the mapped frame or page table is loaded in memory.
    const PRESENT =         1;
    /// Controls whether writes to the mapped frames are allowed.
    const WRITABLE =        1 << 1;
    /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
    const USER = 1 << 2;
    /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
    /// policy is used.
    const WRITE_THROUGH =   1 << 3;
    /// Disables caching for the pointed entry is cacheable.
    const NO_CACHE =        1 << 4;
    /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
    /// the TLB on an address space switch.
    const GLOBAL =          1 << 8;
    /// Forbid execute codes on the page. The NXE bits in EFER msr must be set.
    const NO_EXECUTE = 1 << 63;
  }
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
