// SPDX-License-Identifier: MPL-2.0

//! I/O Memory allocator.

use alloc::vec::Vec;
use core::ops::Range;

use log::{debug, info};
use spin::Once;

use crate::{
    io::io_mem::IoMem,
    mm::{CachePolicy, PageFlags},
    util::range_alloc::RangeAllocator,
};

/// I/O memory allocator that allocates memory I/O access to device drivers.
pub struct IoMemAllocator {
    allocators: Vec<RangeAllocator>,
}

impl IoMemAllocator {
    /// Acquires the I/O memory access for `range`.
    ///
    /// If the range is not available, then the return value will be `None`.
    pub fn acquire(&self, range: Range<usize>) -> Option<IoMem> {
        find_allocator(&self.allocators, &range)?
            .alloc_specific(&range)
            .ok()?;

        debug!("Acquiring MMIO range:{:x?}..{:x?}", range.start, range.end);

        // SAFETY: The created `IoMem` is guaranteed not to access physical memory or system device I/O.
        unsafe { Some(IoMem::new(range, PageFlags::RW, CachePolicy::Uncacheable)) }
    }

    /// Recycles an MMIO range.
    ///
    /// # Safety
    ///
    /// The caller must have ownership of the MMIO region through the `IoMemAllocator::get` interface.
    #[expect(dead_code)]
    pub(in crate::io) unsafe fn recycle(&self, range: Range<usize>) {
        let allocator = find_allocator(&self.allocators, &range).unwrap();

        debug!("Recycling MMIO range:{:x}..{:x}", range.start, range.end);

        allocator.free(range);
    }

    /// Initializes usable memory I/O region.
    ///
    /// # Safety
    ///
    /// User must ensure the range doesn't belong to physical memory or system device I/O.
    unsafe fn new(allocators: Vec<RangeAllocator>) -> Self {
        Self { allocators }
    }
}

/// Builder for `IoMemAllocator`.
///
/// The builder must contains the memory I/O regions that don't belong to the physical memory. Also, OSTD
/// must exclude the memory I/O regions of the system device before building the `IoMemAllocator`.
pub(crate) struct IoMemAllocatorBuilder {
    allocators: Vec<RangeAllocator>,
}

impl IoMemAllocatorBuilder {
    /// Initializes memory I/O region for devices.
    ///
    /// # Safety
    ///
    /// User must ensure the range doesn't belong to physical memory.
    pub(crate) unsafe fn new(ranges: Vec<Range<usize>>) -> Self {
        info!(
            "Creating new I/O memory allocator builder, ranges: {:#x?}",
            ranges
        );
        let mut allocators = Vec::with_capacity(ranges.len());
        for range in ranges {
            allocators.push(RangeAllocator::new(range));
        }
        Self { allocators }
    }

    /// Removes access to a specific memory I/O range.
    ///
    /// All drivers in OSTD must use this method to prevent peripheral drivers from accessing illegal memory I/O range.
    pub(crate) fn remove(&self, range: Range<usize>) {
        let Some(allocator) = find_allocator(&self.allocators, &range) else {
            panic!(
                "Allocator for the system device's MMIO was not found. Range: {:x?}",
                range
            );
        };

        if let Err(err) = allocator.alloc_specific(&range) {
            panic!(
                "An error occurred while trying to remove access to the system device's MMIO. Range: {:x?}. Error: {:?}",
                range, err
            );
        }
    }
}

/// The I/O Memory allocator of the system.
pub static IO_MEM_ALLOCATOR: Once<IoMemAllocator> = Once::new();

/// Initializes the static `IO_MEM_ALLOCATOR` based on builder.
///
/// # Safety
///
/// User must ensure all the memory I/O regions that belong to the system device have been removed by calling the
/// `remove` function.
pub(crate) unsafe fn init(io_mem_builder: IoMemAllocatorBuilder) {
    // SAFETY: The safety is upheld by the caller.
    IO_MEM_ALLOCATOR.call_once(|| unsafe { IoMemAllocator::new(io_mem_builder.allocators) });
}

fn find_allocator<'a>(
    allocators: &'a [RangeAllocator],
    range: &Range<usize>,
) -> Option<&'a RangeAllocator> {
    for allocator in allocators.iter() {
        let allocator_range = allocator.fullrange();
        if allocator_range.start >= range.end || allocator_range.end <= range.start {
            continue;
        }

        return Some(allocator);
    }
    None
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use super::{IoMemAllocator, IoMemAllocatorBuilder};
    use crate::{mm::PAGE_SIZE, prelude::ktest};

    #[expect(clippy::reversed_empty_ranges)]
    #[expect(clippy::single_range_in_vec_init)]
    #[ktest]
    fn illegal_region() {
        let range = vec![0x4000_0000..0x4200_0000];
        let allocator =
            unsafe { IoMemAllocator::new(IoMemAllocatorBuilder::new(range).allocators) };
        assert!(allocator.acquire(0..0).is_none());
        assert!(allocator.acquire(0x4000_0000..0x4000_0000).is_none());
        assert!(allocator.acquire(0x4000_1000..0x4000_0000).is_none());
        assert!(allocator.acquire(usize::MAX..0).is_none());
    }

    #[ktest]
    fn conflict_region() {
        let max_paddr = 0x100_000_000_000; // 16 TB

        let io_mem_region_a = max_paddr..max_paddr + 0x200_0000;
        let io_mem_region_b =
            (io_mem_region_a.end + PAGE_SIZE)..(io_mem_region_a.end + 10 * PAGE_SIZE);
        let range = vec![io_mem_region_a.clone(), io_mem_region_b.clone()];

        let allocator =
            unsafe { IoMemAllocator::new(IoMemAllocatorBuilder::new(range).allocators) };

        assert!(allocator
            .acquire((io_mem_region_a.start - 1)..io_mem_region_a.start)
            .is_none());
        assert!(allocator
            .acquire(io_mem_region_a.start..(io_mem_region_a.start + 1))
            .is_some());

        assert!(allocator
            .acquire((io_mem_region_a.end + 1)..(io_mem_region_b.start - 1))
            .is_none());
        assert!(allocator
            .acquire((io_mem_region_a.end - 1)..(io_mem_region_b.start + 1))
            .is_none());

        assert!(allocator
            .acquire((io_mem_region_a.end - 1)..io_mem_region_a.end)
            .is_some());
        assert!(allocator
            .acquire(io_mem_region_a.end..(io_mem_region_a.end + 1))
            .is_none());
    }
}
