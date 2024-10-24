// SPDX-License-Identifier: MPL-2.0

//! Io Memory access dispatcher.

use alloc::{borrow::ToOwned, vec::Vec};
use core::ops::Range;

use log::{debug, info};
use spin::Once;

use crate::{
    device::io_mem::IoMem,
    mm::{CachePolicy, PageFlags},
    util::vaddr_alloc::VirtAddrAllocator,
};

/// I/O memory dispatcher that allocates I/O access to devices via MMIO.
pub struct IoMemDispatcher {
    allocators: Once<Vec<VirtAddrAllocator>>,
}

impl IoMemDispatcher {
    /// Get the IO memory access with `range`. If the range is not available, then the return value will be None.
    pub fn get(&self, range: Range<usize>) -> Option<IoMem> {
        let mut result = None;
        for allocator in self.allocators.get().unwrap().iter() {
            let allocator_range = allocator.fullrange();
            if allocator_range.start >= range.end || allocator_range.end <= range.start {
                continue;
            }

            let start_addr = range.start;
            let end_addr = range.end;
            debug!("Allocating MMIO range:{:x?}..{:x?}", start_addr, end_addr);
            if allocator.alloc_specific(range).is_err() {
                return None;
            }

            // SAFETY: The created IoMem is guaranteed not to access physical memory or system device I/O
            unsafe {
                result = Some(IoMem::new(
                    start_addr..end_addr,
                    PageFlags::RW,
                    CachePolicy::Uncacheable,
                ))
            }
            break;
        }
        result
    }

    /// Remove access to a specific memory IO range.  
    ///
    /// All drivers in OSTD must use this method to prevent peripheral drivers from accessing illegal memory IO range.
    pub(crate) fn remove(&self, range: Range<usize>) {
        for allocator in self.allocators.get().unwrap().iter() {
            let allocator_range = allocator.fullrange();
            if allocator_range.start >= range.end || allocator_range.end <= range.start {
                continue;
            }

            let start_addr = range.start;
            let end_addr = range.end;
            debug!("Removing MMIO range:{:x}..{:x}", start_addr, end_addr);

            // Remove I/O memory.
            let _ = allocator.alloc_specific(range);
            return;
        }
    }

    /// Recycle an MMIO range.
    #[allow(dead_code)]
    pub(crate) fn recycle(&self, range: Range<usize>) {
        for allocator in self.allocators.get().unwrap().iter() {
            let allocator_range = allocator.fullrange();
            if allocator_range.start >= range.end || allocator_range.end <= range.start {
                continue;
            }

            let start_addr = range.start;
            let end_addr = range.end;
            debug!("Recycling MMIO range:{:x}..{:x}", start_addr, end_addr);

            // Recycle I/O memory.
            allocator.free(range);
            return;
        }
    }

    /// Initialize usable memory I/O region.
    ///
    /// # Safety
    ///
    /// User must ensure the range doesn't belong to physical memory.
    unsafe fn init(&self, ranges: Vec<Range<usize>>) {
        self.allocators.call_once(|| {
            info!(
                "Initialize the IO memory dispatcher with range:{:x?}",
                ranges
            );
            let mut allocators = Vec::with_capacity(ranges.len());
            for range in ranges {
                allocators.push(VirtAddrAllocator::new(range));
            }
            allocators
        });
    }

    const fn new() -> Self {
        Self {
            allocators: Once::new(),
        }
    }
}

/// The I/O Memory dispatcher of the system.
pub static IO_MEM_DISPATCHER: IoMemDispatcher = IoMemDispatcher::new();

pub(crate) fn init() {
    // Initialize the allocator of the IoMemDispatcher based on the system memory info.
    #[cfg(target_arch = "x86_64")]
    init_dispatcher(crate::boot::memory_regions().to_owned());
}

/// This function will initialize the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). Where the area from the top of low memory to 0xffff_ffff, after the top of
/// high memory, is the available MMIO area.
#[cfg(target_arch = "x86_64")]
fn init_dispatcher(mut regions: Vec<crate::boot::memory_region::MemoryRegion>) {
    use align_ext::AlignExt;

    use crate::boot::memory_region::MemoryRegionType;

    assert!(!regions.is_empty());
    regions.sort_by_key(|a| a.base());
    regions.reverse();

    let mut ranges = Vec::new();
    let mut is_tohm_found = false;
    const HIGH_MMIO_SIZE: usize = 0x100_0000;
    const HIGH_MMIO_ALIGN: usize = 0x8_0000_0000;

    for region in regions {
        if region.typ() != MemoryRegionType::Usable {
            continue;
        }

        if !is_tohm_found && (region.base() + region.len()) > u32::MAX as usize {
            // Find the TOHM (Top of High Memory) and initialize High MMIO region.
            // Here, using 16MB (0x100_0000) as the size of High MMIO region.
            //
            // TODO: Update the High MMIO region in runtime.
            is_tohm_found = true;
            // Align start address to HIGH_MMIO_ALIGN
            let mmio_start_addr = (region.base() + region.len()).align_up(HIGH_MMIO_ALIGN);
            ranges.push(mmio_start_addr..(mmio_start_addr + HIGH_MMIO_SIZE));
        } else if (region.base() + region.len()) < u32::MAX as usize {
            // Find the TOLM (Top of Low Memory) and initialize Low MMIO region (TOLM ~ 0xffff_ffff).
            // Align start address to 0x1000_0000
            let mmio_start_addr = (region.base() + region.len()).align_up(0x1000_0000);
            ranges.push(mmio_start_addr..0x1_0000_0000);
            break;
        }
    }

    if !is_tohm_found {
        ranges.push(HIGH_MMIO_ALIGN..(HIGH_MMIO_ALIGN + HIGH_MMIO_SIZE));
    }

    // SAFETY: The range is guaranteed not to access physical memory.
    unsafe {
        IO_MEM_DISPATCHER.init(ranges);
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use super::IoMemDispatcher;
    use crate::{mm::PAGE_SIZE, prelude::ktest};

    #[allow(clippy::reversed_empty_ranges)]
    #[allow(clippy::single_range_in_vec_init)]
    #[ktest]
    fn illegal_region() {
        let dispatcher = IoMemDispatcher::new();
        let range = vec![0x4000_0000..0x4200_0000];
        unsafe {
            dispatcher.init(range);
        }
        assert!(dispatcher.get(0..0).is_none());
        assert!(dispatcher.get(0x4000_0000..0x4000_0000).is_none());
        assert!(dispatcher.get(0x4000_1000..0x4000_0000).is_none());
        assert!(dispatcher.get(usize::MAX..0).is_none());
    }

    #[ktest]
    fn conflict_region() {
        let dispatcher = IoMemDispatcher::new();
        let io_mem_region_a = 0x4000_0000..0x4200_0000;
        let io_mem_region_b =
            (io_mem_region_a.end + PAGE_SIZE)..(io_mem_region_a.end + 10 * PAGE_SIZE);
        let range = vec![io_mem_region_a.clone(), io_mem_region_b.clone()];

        unsafe {
            dispatcher.init(range);
        }

        assert!(dispatcher
            .get((io_mem_region_a.start - 1)..io_mem_region_a.start)
            .is_none());
        assert!(dispatcher
            .get(io_mem_region_a.start..(io_mem_region_a.start + 1))
            .is_some());

        assert!(dispatcher
            .get((io_mem_region_a.end + 1)..(io_mem_region_b.start - 1))
            .is_none());
        assert!(dispatcher
            .get((io_mem_region_a.end - 1)..(io_mem_region_b.start + 1))
            .is_none());

        assert!(dispatcher
            .get((io_mem_region_a.end - 1)..io_mem_region_a.end)
            .is_some());
        assert!(dispatcher
            .get(io_mem_region_a.end..(io_mem_region_a.end + 1))
            .is_none());
    }
}
