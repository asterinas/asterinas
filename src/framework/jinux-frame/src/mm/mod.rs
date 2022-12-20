//! memory management.

pub mod address;
mod frame_allocator;
mod heap_allocator;
mod memory_set;
pub(crate) mod page_table;

use core::sync::atomic::AtomicUsize;

use address::PhysAddr;
use address::VirtAddr;

use crate::x86_64_util;

pub use self::{frame_allocator::*, memory_set::*, page_table::*};

pub(crate) static ORIGINAL_CR3: AtomicUsize = AtomicUsize::new(0);

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

pub(crate) fn init(start: u64, size: u64) {
    heap_allocator::init();
    frame_allocator::init(start as usize, size as usize);
    page_table::init();
    ORIGINAL_CR3.store(
        x86_64_util::get_cr3_raw(),
        core::sync::atomic::Ordering::Relaxed,
    )
}
