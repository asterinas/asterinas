// SPDX-License-Identifier: MPL-2.0

//! The physical page memory allocator.
//!
//! TODO: Decouple it with the frame allocator in [`crate::mm::frame::options`] by
//! allocating pages rather untyped memory from this module.

use alloc::boxed::Box;
use core::{alloc::Layout, ops::Range};

use align_ext::AlignExt;
use log::{info, warn};
use spin::Once;

use crate::{
    mm::{
        page::{meta::PageMeta, ContPages, Page},
        Paddr, PAGE_SIZE,
    },
    sync::SpinLock,
};

/// # PageAlloc trait
///
/// The PageAlloc Trait provides the interface for the page allocator.
/// `PageAlloc` Trait decouples the page allocator implementation from the
/// `ostd`. By corporating with the [`PageAlloc`] trait and
/// [`page_allocator_init_fn`] procedure macro, page allocator's implementation
/// can be decoupled from the OSTD and can be easily replaced to serve
/// designated purposes.
///
/// You can refer to `kernel/libs/aster-page-allocator` for the example
/// implementation.
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

    // TODO(Comments from pr #1137): Refactor the trait to support lock-free
    // design of local page allocation cache. Specifically, change all the
    // signatures to `&self` and require the implementor to use their own
    // synchronization primitives to manage their locking scheme.

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

/// The global page allocator, described by the `PageAlloc` trait.
#[export_name = "PAGE_ALLOCATOR"]
pub static PAGE_ALLOCATOR: Once<SpinLock<Box<dyn PageAlloc>>> = Once::new();

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

pub(crate) fn init() {
    let allocator: Box<dyn PageAlloc>;
    unsafe {
        extern "Rust" {
            fn __ostd_page_allocator_init_fn() -> Box<dyn PageAlloc>;
        }
        allocator = __ostd_page_allocator_init_fn();
    }
    PAGE_ALLOCATOR.call_once(|| SpinLock::new(allocator));
}

/// The bootstrapping phase page allocator.
pub(crate) struct BootstrapFrameAllocator {
    // memory region idx: The index for the global memory region indicates the
    // current memory region in use, facilitating rapid boot page allocation.
    mem_region_idx: usize,
    // frame cursor: The cursor for the frame which is the next frame to be
    // allocated.
    frame_cursor: usize,
}

pub(in crate::mm) static BOOTSTRAP_PAGE_ALLOCATOR: Once<SpinLock<BootstrapFrameAllocator>> =
    Once::new();

impl BootstrapFrameAllocator {
    pub fn new() -> Self {
        // Get the first frame for allocation
        let mut first_idx = 0;
        let mut first_frame = 0;
        let regions = crate::boot::memory_regions();
        for (i, region) in regions.iter().enumerate() {
            if region.typ() == crate::boot::memory_region::MemoryRegionType::Usable {
                // Make the memory region page-aligned, and skip if it is too small.
                let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
                let end = region
                    .base()
                    .checked_add(region.len())
                    .unwrap()
                    .align_down(PAGE_SIZE)
                    / PAGE_SIZE;
                log::debug!(
                    "Found usable region, start:{:x}, end:{:x}",
                    region.base(),
                    region.base() + region.len()
                );
                if end <= start {
                    continue;
                } else {
                    first_idx = i;
                    first_frame = start;
                    break;
                }
            }
        }
        Self {
            mem_region_idx: first_idx,
            frame_cursor: first_frame,
        }
    }
}

impl PageAlloc for BootstrapFrameAllocator {
    fn add_free_pages(&mut self, _range: Range<usize>) {
        warn!("BootstrapFrameAllocator does not need to add frames");
    }

    fn alloc(&mut self, _layout: Layout) -> Option<Paddr> {
        warn!("BootstrapFrameAllocator does not support to allocate memory described by range");
        None
    }

    fn alloc_page(&mut self, _align: usize) -> Option<Paddr> {
        let frame = self.frame_cursor;
        // Update idx and cursor
        let regions = crate::boot::memory_regions();
        self.frame_cursor += 1;
        loop {
            let region = regions[self.mem_region_idx];
            if region.typ() == crate::boot::memory_region::MemoryRegionType::Usable {
                let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
                let end = region
                    .base()
                    .checked_add(region.len())
                    .unwrap()
                    .align_down(PAGE_SIZE)
                    / PAGE_SIZE;
                if end <= start {
                    self.mem_region_idx += 1;
                    continue;
                }
                if self.frame_cursor < start {
                    self.frame_cursor = start;
                }
                if self.frame_cursor >= end {
                    self.mem_region_idx += 1;
                } else {
                    break;
                }
            } else {
                self.mem_region_idx += 1;
            }
            if self.mem_region_idx >= regions.len() {
                panic!("no more usable memory regions for boot page table");
            }
        }
        Some(frame * PAGE_SIZE)
    }

    fn dealloc(&mut self, _addr: Paddr, _nr_pages: usize) {
        panic!("BootstrapFrameAllocator does support frames deallocation!");
    }

    fn total_mem(&self) -> usize {
        warn!("BootstrapFrameAllocator does not support to calculate total memory");
        0
    }

    fn free_mem(&self) -> usize {
        warn!("BootstrapFrameAllocator does not support to calculate free memory");
        0
    }
}

pub(crate) fn bootstrap_init() {
    info!("Initializing the bootstrap page allocator");
    BOOTSTRAP_PAGE_ALLOCATOR.call_once(|| SpinLock::new(BootstrapFrameAllocator::new()));
}
