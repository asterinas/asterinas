// SPDX-License-Identifier: MPL-2.0

//! The physical page memory allocator.
//!
//! TODO: Decouple it with the frame allocator in [`crate::mm::frame::options`] by
//! allocating pages rather untyped memory from this module.

use alloc::boxed::Box;
use core::alloc::Layout;

use align_ext::AlignExt;
use buddy_system_allocator::FrameAllocator;
use log::info;
use spin::Once;

use crate::{
    boot::memory_region::MemoryRegionType,
    mm::{Paddr, PAGE_SIZE},
    sync::SpinLock,
};

pub trait PageAlloc: Sync + Send {
    /// Add a range of **frame number** [start, end) to the allocator
    ///
    /// Warning! May lead to panic when afterwards allocation while using
    /// out-of `ostd`
    fn add_frame(&mut self, start: usize, end: usize);

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
    /// Warning! May lead to panic when afterwards allocation while using
    /// out-of `ostd`
    fn dealloc(&mut self, addr: Paddr, nr_pages: usize);

    /// Returns the total number of pages available for allocation.
    fn total_pages(&self) -> usize;

    /// Returns the number of currently free pages.
    fn free_pages(&self) -> usize;
}

#[export_name = "PAGE_ALLOCATOR"]
pub(in crate::mm) static PAGE_ALLOCATOR: Once<SpinLock<Box<dyn PageAlloc>>> = Once::new();

impl PageAlloc for FrameAllocator<32> {
    fn add_frame(&mut self, start: usize, end: usize) {
        FrameAllocator::add_frame(self, start, end)
    }

    fn alloc(&mut self, layout: Layout) -> Option<Paddr> {
        FrameAllocator::alloc_aligned(self, layout)
            .map(|idx| idx * PAGE_SIZE)
    }

    fn dealloc(&mut self, addr: Paddr, nr_pages: usize) {
        FrameAllocator::dealloc(self, addr / PAGE_SIZE, nr_pages)
    }

    // Refactor buddy frame allocator to read the following information
    fn total_pages(&self) -> usize {
        0
    }

    fn free_pages(&self) -> usize {
        0
    }
}

// /// Allocate a single page.
// pub(crate) fn alloc_single<M: PageMeta>() -> Option<Page<M>> {
//     PAGE_ALLOCATOR.get().unwrap().lock().alloc(1).map(|idx| {
//         let paddr = idx * PAGE_SIZE;
//         Page::<M>::from_unused(paddr)
//     })
// }

// /// Allocate a contiguous range of pages of a given length in bytes.
// ///
// /// # Panics
// ///
// /// The function panics if the length is not base-page-aligned.
// pub(crate) fn alloc_contiguous<M: PageMeta>(len: usize) -> Option<ContPages<M>> {
//     assert!(len % PAGE_SIZE == 0);
//     PAGE_ALLOCATOR
//         .get()
//         .unwrap()
//         .lock()
//         .alloc(len / PAGE_SIZE)
//         .map(|start| ContPages::from_unused(start * PAGE_SIZE..start * PAGE_SIZE + len))
// }

// /// Allocate pages.
// ///
// /// The allocated pages are not guarenteed to be contiguous.
// /// The total length of the allocated pages is `len`.
// ///
// /// # Panics
// ///
// /// The function panics if the length is not base-page-aligned.
// pub(crate) fn alloc<M: PageMeta>(len: usize) -> Option<Vec<Page<M>>> {
//     assert!(len % PAGE_SIZE == 0);
//     let nframes = len / PAGE_SIZE;
//     let mut allocator = PAGE_ALLOCATOR.get().unwrap().lock();
//     let mut vector = Vec::new();
//     for _ in 0..nframes {
//         let paddr = allocator.alloc(1)? * PAGE_SIZE;
//         let page = Page::<M>::from_unused(paddr);
//         vector.push(page);
//     }
//     Some(vector)
// }

pub(crate) fn init() {
    let regions = crate::boot::memory_regions();
    let mut allocator = Box::new(FrameAllocator::<32>::new());
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
            allocator.add_frame(start, end);
            total += (end - start) * PAGE_SIZE;
            info!(
                "Found usable region, start:{:x}, end:{:x}",
                region.base(),
                region.base() + region.len()
            );
        }
    }
    let counting_allocator = CountingFrameAllocator::new(allocator, total);
    PAGE_ALLOCATOR.call_once(|| SpinLock::new(counting_allocator));
}
