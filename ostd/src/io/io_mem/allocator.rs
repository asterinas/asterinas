// SPDX-License-Identifier: MPL-2.0

//! I/O Memory allocator.

use alloc::vec::Vec;
use core::ops::Range;

use log::{debug, info};
use spin::Once;

use crate::{
    io::io_mem::{Insensitive, IoMem, Sensitive},
    mm::{CachePolicy, PageFlags},
    util::range_alloc::RangeAllocator,
};

/// I/O memory allocator that allocates memory I/O access to device drivers.
pub(super) struct IoMemAllocator {
    allocators: Vec<RangeAllocator>,
}

impl IoMemAllocator {
    /// Acquires `range` for insensitive MMIO with the specified cache policy.
    ///
    /// If the range is not available, then the return value will be `None`.
    pub(super) fn acquire(
        &self,
        range: Range<usize>,
        cache_policy: CachePolicy,
    ) -> Option<IoMem<Insensitive>> {
        debug!(
            "Try to acquire security-insensitive MMIO range: {:#x?} with cache policy: {:?}",
            range, cache_policy
        );

        find_allocator(&self.allocators, &range)?
            .alloc_specific(&range)
            .ok()?;

        // SAFETY:
        // 1. The `IoMemAllocator` instance is built from an
        //    `IoMemAllocatorBuilder` instance, which is constructed after the
        //    kernel page table is activated.
        // 2. The created `IoMem` is guaranteed not to access physical memory or
        //    system device I/O.
        unsafe { Some(IoMem::new(range, PageFlags::RW, cache_policy)) }
    }

    /// Recycles an MMIO range.
    ///
    /// # Safety
    ///
    /// The caller must have ownership of the MMIO region through the `IoMemAllocator::get` interface.
    #[expect(dead_code)]
    pub(super) unsafe fn recycle(&self, range: Range<usize>) {
        debug!("Recycling MMIO range: {:#x?}", range);

        let allocator = find_allocator(&self.allocators, &range).unwrap();

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
    /// 1. This function must be called at most once.
    /// 2. This function must be called after the kernel page table is activated
    ///    on the bootstrapping processor.
    /// 3. The caller must ensure the range doesn't belong to physical memory.
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

    /// Reserves `range` from the allocator for sensitive MMIO with the specified cache policy.
    ///
    /// # Panics
    ///
    /// This function will panic if the specified range is not available.
    #[cfg_attr(target_arch = "loongarch64", expect(unused))]
    pub(crate) fn reserve(
        &self,
        range: Range<usize>,
        cache_policy: CachePolicy,
    ) -> IoMem<Sensitive> {
        debug!(
            "Try to reserve security-sensitive MMIO range: {:#x?} with cache policy: {:?}",
            range, cache_policy
        );

        self.remove(range.start..range.end);

        // SAFETY:
        // 1. The `IoMemAllocatorBuilder` instance is constructed after the
        //    kernel page table is activated.
        // 2. The range falls within I/O memory area and does not overlap
        //    with other system devices' I/O memory.
        unsafe { IoMem::new(range, PageFlags::RW, cache_policy) }
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
pub(super) static IO_MEM_ALLOCATOR: Once<IoMemAllocator> = Once::new();

/// Initializes the static `IO_MEM_ALLOCATOR` based on builder.
///
/// # Safety
///
/// User must ensure all the memory I/O regions that belong to the system device have been removed by calling the
/// `remove` function.
pub(in crate::io) unsafe fn init(io_mem_builder: IoMemAllocatorBuilder) {
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
        assert!(allocator.acquire(usize::MAX..0).is_none());

        assert!(allocator.acquire(0x4000_0000..0x4000_0000).is_none());
        assert!(allocator.acquire(0x4000_1000..0x4000_1000).is_none());
        assert!(allocator.acquire(0x41ff_0000..0x41ff_0000).is_none());
        assert!(allocator.acquire(0x4200_0000..0x4200_0000).is_none());

        assert!(allocator.acquire(0x4000_1000..0x4000_0000).is_none());
        assert!(allocator.acquire(0x4000_2000..0x4000_1000).is_none());
        assert!(allocator.acquire(0x41ff_f000..0x41ff_e000).is_none());
        assert!(allocator.acquire(0x4200_0000..0x41ff_f000).is_none());
    }

    #[ktest]
    fn conflict_region() {
        let max_paddr = 0x100_000_000_000; // 16 TiB

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
