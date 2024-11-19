// SPDX-License-Identifier: MPL-2.0

//! The physical page memory allocator.
//!
//! TODO: Decouple it with the frame allocator in [`crate::mm::frame::options`] by
//! allocating pages rather untyped memory from this module.

use alloc::boxed::Box;
use core::{
    alloc::Layout,
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
};

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
/// `ostd`. By corporating with the [`PageAlloc`] trait and the
/// [`GlobalPageAllocator::inject`] function, page allocator's implementation
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
    fn add_free_pages(&self, range: Range<usize>);

    /// Allocates a contiguous range of pages described by the layout.
    ///
    /// # Panics
    ///
    /// The function panics if the layout.size is not base-page-aligned or
    /// if the layout.align is less than the PAGE_SIZE.
    fn alloc(&self, layout: Layout) -> Option<Paddr>;

    /// Allocates one page with specific alignment
    ///
    /// # Panics
    ///
    /// The function panics if the align is not a power-of-two
    fn alloc_page(&self, align: usize) -> Option<Paddr> {
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
    fn dealloc(&self, addr: Paddr, nr_pages: usize);

    /// Returns the total number of bytes managed by the allocator.
    fn total_mem(&self) -> usize;

    /// Returns the total number of bytes available for allocation.
    fn free_mem(&self) -> usize;
}

/// The bootstrapping phase page allocator.
pub(crate) struct BootFrameAllocator {
    // memory region idx: The index for the global memory region indicates the
    // current memory region in use, facilitating rapid boot page allocation.
    mem_region_idx: usize,
    // frame cursor: The cursor for the frame which is the next frame to be
    // allocated.
    frame_cursor: usize,
}

/// The locked version of the bootstrapping phase page allocator.
/// Import [`SpinLock`] to get the inner mutable reference, catering to
/// the requirement of the `PageAlloc` trait.
pub(crate) struct LockedBootFrameAllocator {
    // The bootstrap frame allocator with a spin lock.
    allocator: SpinLock<BootFrameAllocator>,
}

impl BootFrameAllocator {
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

    /// Allocate pages for the bootstrapping phase.
    ///
    /// # Notice
    ///
    /// The align **MUST BE** 4KB, otherwise it will panic.
    pub fn alloc_pages(&mut self, count: usize) -> Option<Paddr> {
        let frame: usize;
        // Update idx and cursor
        let regions = crate::boot::memory_regions();
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
                if self.frame_cursor + count >= end {
                    self.mem_region_idx += 1;
                } else {
                    frame = self.frame_cursor;
                    self.frame_cursor += count;
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
}

impl LockedBootFrameAllocator {
    pub fn new() -> Self {
        Self {
            allocator: SpinLock::new(BootFrameAllocator::new()),
        }
    }
}

impl PageAlloc for LockedBootFrameAllocator {
    fn add_free_pages(&self, _range: Range<usize>) {
        warn!("BootFrameAllocator does not need to add frames");
    }

    fn alloc(&self, layout: Layout) -> Option<Paddr> {
        self.allocator
            .disable_irq()
            .lock()
            .alloc_pages(layout.size() / PAGE_SIZE)
    }

    fn dealloc(&self, _addr: Paddr, _nr_pages: usize) {
        warn!("BootFrameAllocator does support frames deallocation!");
    }

    fn total_mem(&self) -> usize {
        warn!("BootFrameAllocator does not support to calculate total memory");
        0
    }

    fn free_mem(&self) -> usize {
        warn!("BootFrameAllocator does not support to calculate free memory");
        0
    }
}

/// Global page allocator that wraps the default page allocator and the injected
/// page allocator.
///
/// The injected page allocator is implemented out of `ostd` with safe code and
/// is used to replace the default page allocator.
pub struct GlobalPageAllocator {
    /// Whether the page allocator is injected. If true, the page allocator is
    /// injected; otherwise, it is [`LockedBootFrameAllocator`] by default.
    is_injected: AtomicBool,

    /// The default page allocator.
    default_allocator: LockedBootFrameAllocator,

    /// The injected page allocator.
    injected_allocator: Once<Box<dyn PageAlloc>>,
}

/// The global page allocator, described by the `PageAlloc` trait.
#[export_name = "PAGE_ALLOCATOR"]
pub static PAGE_ALLOCATOR: Once<GlobalPageAllocator> = Once::new();

impl GlobalPageAllocator {
    /// Creates a new global page allocator.
    pub fn new() -> Self {
        Self {
            is_injected: AtomicBool::new(false),
            default_allocator: LockedBootFrameAllocator::new(),
            injected_allocator: Once::new(),
        }
    }

    /// Injects a page allocator, which is used to replace the default page
    /// allocator.
    pub fn inject(&self, allocator: Box<dyn PageAlloc>) {
        self.injected_allocator.call_once(|| allocator);
        self.is_injected.store(true, Ordering::Relaxed);
        info!("Inject the page allocator");
    }

    /// Checks whether the page allocator is injected.
    pub(crate) fn check_injected(&self) -> bool {
        self.is_injected.load(Ordering::Relaxed)
    }
}

impl Default for GlobalPageAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl PageAlloc for GlobalPageAllocator {
    fn add_free_pages(&self, range: Range<usize>) {
        if self.check_injected() {
            self.injected_allocator.get().unwrap().add_free_pages(range);
        } else {
            self.default_allocator.add_free_pages(range);
        }
    }

    fn alloc(&self, layout: Layout) -> Option<Paddr> {
        if self.check_injected() {
            self.injected_allocator.get().unwrap().alloc(layout)
        } else {
            self.default_allocator.alloc(layout)
        }
    }

    /// Deallocate a contiguous range of pages.
    /// The caller should provide the physical address of the first page and the
    /// number of pages to deallocate.
    ///
    /// # Notice
    ///
    /// 1. If the page allocator is injected, the injected allocator will be
    ///    used; otherwise, the default allocator will be used.
    /// 2. The default allocator does not support deallocation. Since the
    ///    deallocated pages' meta will be set to ['FreeMeta'], the injected
    ///    allocator will update deallocation context accordingly.
    fn dealloc(&self, addr: Paddr, nr_pages: usize) {
        if self.check_injected() {
            self.injected_allocator
                .get()
                .unwrap()
                .dealloc(addr, nr_pages);
        } else {
            self.default_allocator.dealloc(addr, nr_pages);
        }
    }

    fn total_mem(&self) -> usize {
        if self.check_injected() {
            self.injected_allocator.get().unwrap().total_mem()
        } else {
            self.default_allocator.total_mem()
        }
    }

    fn free_mem(&self) -> usize {
        if self.check_injected() {
            self.injected_allocator.get().unwrap().free_mem()
        } else {
            self.default_allocator.free_mem()
        }
    }
}

/// Allocate a single page.
///
/// The metadata of the page is initialized with the given metadata.
///
/// # Notice
///
/// 1. Should be called after the [`mm::init_page_meta()`] is finished.
/// 2. If the page allocator is injected, the injected allocator will be
///    used; otherwise, the default allocator will be used.
/// 3. While using the default allocator, The align **MUST BE** 4KB,
///    otherwise it will panic.
pub(crate) fn alloc_single<M: PageMeta>(align: usize, metadata: M) -> Option<Page<M>> {
    PAGE_ALLOCATOR
        .get()
        .unwrap()
        .alloc_page(align)
        .map(|paddr| Page::from_free(paddr, metadata))
}

/// Allocate a contiguous range of pages of a given length in bytes.
///
/// The caller must provide a closure to initialize metadata for all the
/// pages. The closure receives the physical address of the page and
/// returns the metadata, which is similar to [`core::array::from_fn`].
///
/// # Notice
///
/// 1. The function panics if the layout is not base-page-aligned.
/// 2. Should be called after the [`mm::init_page_meta()`] is finished.
/// 3. If the page allocator is injected, the injected allocator will be
///    used; otherwise, the default allocator will be used.
/// 4. While using the default allocator, The align **MUST BE** 4KB,
///    otherwise it will panic.
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
        .alloc(layout)
        .map(|begin_paddr| {
            ContPages::from_free(begin_paddr..begin_paddr + layout.size(), metadata_fn)
        })
}

pub(crate) fn bootstrap_init() {
    info!("Initializing the bootstrap page allocator");
    PAGE_ALLOCATOR.call_once(GlobalPageAllocator::new);
}
