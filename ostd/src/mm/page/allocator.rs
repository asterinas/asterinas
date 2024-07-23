// SPDX-License-Identifier: MPL-2.0 OR MIT

//! The physical page memory allocator.

//！ To be specific, the original source code is from
//！ [buddy_system_allocator](https://github.com/rcore-os/buddy_system_allocator),
//！ which licensed under the following license.
//！
//！ SPDX-License-Identifier: MIT
//！
//！ Copyright (c) 2019-2020 Jiajie Chen
//！
//！ We make the following new changes:
//！ * Implement `PageAlloc` trait for `BuddyFrameAllocator`.
//!  * Add statistics for the total memory and free memory.
//！ * Refactor API to differentiate count and size of frames.
//！
//！ These changes are released under the following license:
//！
//！ SPDX-License-Identifier: MPL-2.0
//!
//! TODO: Decouple it with the frame allocator in [`crate::mm::frame::options`] by
//! allocating pages rather untyped memory from this module.

use alloc::{boxed::Box, collections::btree_set::BTreeSet};
use core::{alloc::Layout, array, cmp::min, ops::Range};

use align_ext::AlignExt;
use log::info;
use spin::Once;

use crate::{
    boot::memory_region::MemoryRegionType,
    mm::{
        page::{meta::PageMeta, ContPages, Page},
        Paddr, PAGE_SIZE,
    },
    sync::SpinLock,
};

pub trait PageAlloc: Sync + Send {
    /// Add a range of free pages, described by the **frame number**
    /// [start, end), for the allocator to manage.
    ///
    /// Warning! May lead to panic when afterwards allocation while using
    /// out-of `ostd`
    fn add_free_pages(&mut self, range: Range<usize>);

    /// Allocates a contiguous range of pages described by the layout.
    ///
    /// # Panics
    ///
    /// The function panics if the layout.size is not base-page-aligned or
    /// if the layout.align is less than the PAGE_SIZE.
    fn alloc(&mut self, layout: Layout) -> Option<Paddr>;

    /// Allocates one page with specific alignment
    ///
    /// # Panics
    ///
    /// The function panics if the align is not a power-of-two
    fn alloc_page(&mut self, align: usize) -> Option<Paddr> {
        // CHeck whether the align is always a power-of-two
        assert!(align.is_power_of_two());
        let alignment = core::cmp::max(align, PAGE_SIZE);
        self.alloc(Layout::from_size_align(PAGE_SIZE, alignment).unwrap())
    }

    /// Deallocates a specified number of consecutive pages.
    ///
    /// # Warning
    ///
    /// In `ostd`, the correctness of the allocation / deallocation is
    /// guaranteed by the meta system ( [`crate::mm::page::meta`] ), while the
    /// page allocator is only responsible for managing the allocation
    /// metadata. The meta system can only be used within the `ostd` crate.
    ///
    /// Therefore, deallocating pages out-of `ostd` without coordination with
    /// the meta system may lead to unexpected behavior, such as panics during
    /// afterwards allocation.
    fn dealloc(&mut self, addr: Paddr, nr_pages: usize);

    /// Returns the total number of bytes managed by the allocator.
    fn total_mem(&self) -> usize;

    /// Returns the total number of bytes available for allocation.
    fn free_mem(&self) -> usize;
}

#[export_name = "PAGE_ALLOCATOR"]
pub(in crate::mm) static PAGE_ALLOCATOR: Once<SpinLock<Box<dyn PageAlloc>>> = Once::new();

/// Allocate a single page.
///
/// The metadata of the page is initialized with the given metadata.
pub(crate) fn alloc_single<M: PageMeta>(align: usize, metadata: M) -> Option<Page<M>> {
    PAGE_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc_page(align)
        .map(|paddr| Page::from_unused(paddr, metadata))
}

/// Allocate a contiguous range of pages of a given length in bytes.
///
/// The caller must provide a closure to initialize metadata for all the pages.
/// The closure receives the physical address of the page and returns the
/// metadata, which is similar to [`core::array::from_fn`].
///
/// # Panics
///
/// The function panics if the length is not base-page-aligned.
pub(crate) fn alloc_contiguous<M: PageMeta, F>(
    layout: Layout,
    metadata_fn: F,
) -> Option<ContPages<M>>
where
    F: FnMut(Paddr) -> M,
{
    assert!(layout.size() % PAGE_SIZE == 0);
    PAGE_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(layout)
        .map(|begin_paddr| {
            ContPages::from_unused(begin_paddr..begin_paddr + layout.size(), metadata_fn)
        })
}

/// Buddy Frame allocator is a frame allocator based on buddy system, which
/// allocates memory in power-of-two sizes.
///
/// The max order of the allocator is specified via the const generic parameter
/// `ORDER`. The frame allocator will only be able to allocate ranges of size
/// up to 2<sup>ORDER</sup>, out of a total range of size at most 2<sup>ORDER +
/// 1</sup> - 1.
pub struct BuddyFrameAllocator<const ORDER: usize = 32> {
    // buddy system with max order of ORDER
    free_list: [BTreeSet<usize>; ORDER],

    // statistics
    allocated: usize,
    total: usize,
}

pub(crate) fn prev_power_of_two(num: usize) -> usize {
    1 << (8 * (size_of::<usize>()) - num.leading_zeros() as usize - 1)
}

impl<const ORDER: usize> BuddyFrameAllocator<ORDER> {
    /// Create an empty frame allocator
    pub fn new() -> Self {
        Self {
            free_list: array::from_fn(|_| BTreeSet::default()),
            allocated: 0,
            total: 0,
        }
    }

    /// Add a range of free pages, described by the **frame number**
    /// [start, end), for the allocator to manage.
    fn add_free_pages(&mut self, range: Range<usize>) {
        let start = range.start;
        let end = range.end;
        assert!(start <= end);

        let mut total = 0;
        let mut current_start = start;

        while current_start < end {
            let lowbit = if current_start > 0 {
                current_start & (!current_start + 1)
            } else {
                32
            };
            let size = min(
                min(lowbit, prev_power_of_two(end - current_start)),
                1 << (ORDER - 1),
            );
            total += size;

            self.free_list[size.trailing_zeros() as usize].insert(current_start);
            current_start += size;
        }

        self.total += total;
    }

    /// Allocate a range of frames from the allocator, returning the first frame of the allocated
    /// range.
    pub fn alloc(&mut self, count: usize) -> Option<usize> {
        self.alloc_power_of_two(count.next_power_of_two())
    }

    /// Allocate a range of frames of the given size from the allocator. The size must be a power of
    /// two. The allocated range will have alignment equal to the size.
    fn alloc_power_of_two(&mut self, count: usize) -> Option<usize> {
        let class = count.trailing_zeros() as usize;
        for i in class..self.free_list.len() {
            // Find the first non-empty size class
            if !self.free_list[i].is_empty() {
                // Split buffers
                for j in (class + 1..i + 1).rev() {
                    if let Some(block_ref) = self.free_list[j].iter().next() {
                        let block = *block_ref;
                        self.free_list[j - 1].insert(block + (1 << (j - 1)));
                        self.free_list[j - 1].insert(block);
                        self.free_list[j].remove(&block);
                    } else {
                        return None;
                    }
                }

                let result = self.free_list[class].iter().next().clone();
                if let Some(result_ref) = result {
                    let result = *result_ref;
                    self.free_list[class].remove(&result);
                    self.allocated += count;
                    return Some(result);
                } else {
                    return None;
                }
            }
        }
        None
    }

    /// Deallocate a range of frames [frame, frame+count) from the frame allocator.
    ///
    /// The range should be exactly the same when it was allocated, as in heap allocator
    pub fn dealloc(&mut self, start_frame: usize, count: usize) {
        self.dealloc_power_of_two(start_frame, count.next_power_of_two())
    }

    /// Deallocate a range of frames with the given size from the allocator. The size must be a
    /// power of two.
    fn dealloc_power_of_two(&mut self, start_frame: usize, count: usize) {
        let class = count.trailing_zeros() as usize;

        // Merge free buddy lists
        let mut current_ptr = start_frame;
        let mut current_class = class;
        while current_class < self.free_list.len() {
            let buddy = current_ptr ^ (1 << current_class);
            if self.free_list[current_class].remove(&buddy) == true {
                // Free buddy found
                current_ptr = min(current_ptr, buddy);
                current_class += 1;
            } else {
                self.free_list[current_class].insert(current_ptr);
                break;
            }
        }

        self.allocated -= count;
    }
}

impl PageAlloc for BuddyFrameAllocator<32> {
    fn add_free_pages(&mut self, range: Range<usize>) {
        BuddyFrameAllocator::add_free_pages(self, range)
    }

    fn alloc(&mut self, layout: Layout) -> Option<Paddr> {
        assert!(layout.size() & (PAGE_SIZE - 1) == 0);
        BuddyFrameAllocator::alloc(self, layout.size() / PAGE_SIZE).map(|idx| idx * PAGE_SIZE)
    }

    fn dealloc(&mut self, addr: Paddr, nr_pages: usize) {
        BuddyFrameAllocator::dealloc(self, addr / PAGE_SIZE, nr_pages)
    }

    fn total_mem(&self) -> usize {
        self.total * PAGE_SIZE
    }

    fn free_mem(&self) -> usize {
        (self.total - self.allocated) * PAGE_SIZE
    }
}

pub(crate) fn init() {
    let regions = crate::boot::memory_regions();
    let mut allocator = Box::new(BuddyFrameAllocator::<32>::new());
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Usable {
            // Make the memory region page-aligned, and skip if it is too small.
            let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
            let region_end = region.base().checked_add(region.len()).unwrap();
            let end = region_end.align_down(PAGE_SIZE) / PAGE_SIZE;
            if end <= start {
                continue;
            }
            // Add global free pages to the frame allocator.
            allocator.add_free_pages(Range { start, end });
            info!(
                "Found usable region, start:{:x}, end:{:x}",
                region.base(),
                region.base() + region.len()
            );
        }
    }
    PAGE_ALLOCATOR.call_once(|| SpinLock::new(allocator));
}
