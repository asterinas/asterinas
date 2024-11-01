// SPDX-License-Identifier: MPL-2.0

//! The physical page memory allocator.
//!
//! TODO: Decouple it with the frame allocator in [`crate::mm::frame::options`] by
//! allocating pages rather untyped memory from this module.

use alloc::vec::Vec;

use align_ext::AlignExt;
use buddy_system_allocator::FrameAllocator;
use log::info;
use spin::Once;

use super::{cont_pages::ContPages, meta::PageMeta, Page};
use crate::{
    boot::memory_region::MemoryRegionType,
    mm::{Paddr, PAGE_SIZE},
    sync::SpinLock,
};

/// FrameAllocator with a counter for allocated memory
pub(in crate::mm) struct CountingFrameAllocator {
    allocator: FrameAllocator,
    total: usize,
    allocated: usize,
}

impl CountingFrameAllocator {
    pub fn new(allocator: FrameAllocator, total: usize) -> Self {
        CountingFrameAllocator {
            allocator,
            total,
            allocated: 0,
        }
    }

    pub fn alloc(&mut self, count: usize) -> Option<usize> {
        match self.allocator.alloc(count) {
            Some(value) => {
                self.allocated += count * PAGE_SIZE;
                Some(value)
            }
            None => None,
        }
    }

    pub fn dealloc(&mut self, start_frame: usize, count: usize) {
        self.allocator.dealloc(start_frame, count);
        self.allocated -= count * PAGE_SIZE;
    }

    pub fn mem_total(&self) -> usize {
        self.total
    }

    pub fn mem_available(&self) -> usize {
        self.total - self.allocated
    }
}

pub(in crate::mm) static PAGE_ALLOCATOR: Once<SpinLock<CountingFrameAllocator>> = Once::new();

/// Allocate a single page.
///
/// The metadata of the page is initialized with the given metadata.
pub(crate) fn alloc_single<M: PageMeta>(metadata: M) -> Option<Page<M>> {
    PAGE_ALLOCATOR
        .get()
        .unwrap()
        .disable_irq()
        .lock()
        .alloc(1)
        .map(|idx| {
            let paddr = idx * PAGE_SIZE;
            Page::from_unused(paddr, metadata)
        })
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
pub(crate) fn alloc_contiguous<M: PageMeta, F>(len: usize, metadata_fn: F) -> Option<ContPages<M>>
where
    F: FnMut(Paddr) -> M,
{
    assert!(len % PAGE_SIZE == 0);
    PAGE_ALLOCATOR
        .get()
        .unwrap()
        .disable_irq()
        .lock()
        .alloc(len / PAGE_SIZE)
        .map(|start| {
            ContPages::from_unused(start * PAGE_SIZE..start * PAGE_SIZE + len, metadata_fn)
        })
}

/// Allocate pages.
///
/// The allocated pages are not guaranteed to be contiguous.
/// The total length of the allocated pages is `len`.
///
/// The caller must provide a closure to initialize metadata for all the pages.
/// The closure receives the physical address of the page and returns the
/// metadata, which is similar to [`core::array::from_fn`].
///
/// # Panics
///
/// The function panics if the length is not base-page-aligned.
pub(crate) fn alloc<M: PageMeta, F>(len: usize, mut metadata_fn: F) -> Option<Vec<Page<M>>>
where
    F: FnMut(Paddr) -> M,
{
    assert!(len % PAGE_SIZE == 0);
    let nframes = len / PAGE_SIZE;
    let mut allocator = PAGE_ALLOCATOR.get().unwrap().disable_irq().lock();
    let mut vector = Vec::new();
    for _ in 0..nframes {
        let paddr = allocator.alloc(1)? * PAGE_SIZE;
        let page = Page::<M>::from_unused(paddr, metadata_fn(paddr));
        vector.push(page);
    }
    Some(vector)
}

pub(crate) fn init() {
    let regions = crate::boot::memory_regions();
    let mut total: usize = 0;
    let mut allocator = FrameAllocator::<32>::new();
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
