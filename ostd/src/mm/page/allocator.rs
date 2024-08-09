// SPDX-License-Identifier: MPL-2.0

//! The physical page memory allocator.
//!
//! TODO: Decouple it with the frame allocator in [`crate::mm::frame::options`] by
//! allocating pages rather untyped memory from this module.

use alloc::boxed::Box;
use core::alloc::Layout;

use align_ext::AlignExt;
use log::{info, warn};
use spin::Once;

use crate::{
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

    /// Returns the total memory size in **bytes** that is
    /// available for allocation.
    fn total_mem(&self) -> usize;

    /// Returns the free memory size in **bytes** that is
    /// available for allocation.
    fn free_mem(&self) -> usize;
}

#[export_name = "PAGE_ALLOCATOR"]
pub(in crate) static PAGE_ALLOCATOR: Once<SpinLock<Box<dyn PageAlloc>>> = Once::new();

pub(crate) fn init() {
    let allocator: Box<dyn PageAlloc>;
    unsafe {
        extern "Rust" {
            fn __ostd_page_allocator_init() -> Box<dyn PageAlloc>;
        }
        allocator = __ostd_page_allocator_init();
    }
    PAGE_ALLOCATOR.call_once(|| SpinLock::new(allocator));
}

pub(crate) struct BootstrapFrameAllocator {
    // memory region idx: The index for the global memory region indicates the
    // current memory region in use, facilitating rapid boot page allocation.
    mem_region_idx: usize,
    // frame cursor: The cursor for the frame which is the next frame to be
    // allocated.
    frame_cursor: usize,
}

#[export_name = "BOOTSTRAP_PAGE_ALLOCATOR"]
pub(in crate::mm) static BOOTSTRAP_PAGE_ALLOCATOR: Once<SpinLock<BootstrapFrameAllocator>> =
    Once::new();

impl BootstrapFrameAllocator {
    pub fn new() -> Self {
        // Get the first frame for allocation
        let mut first_idx = 0;
        let mut first_frame = 0;
        let regions = crate::boot::memory_regions();
        for i in 0..regions.len() {
            if regions[i].typ() == crate::boot::memory_region::MemoryRegionType::Usable {
                // Make the memory region page-aligned, and skip if it is too small.
                let start = regions[i].base().align_up(PAGE_SIZE) / PAGE_SIZE;
                let end = regions[i]
                    .base()
                    .checked_add(regions[i].len())
                    .unwrap()
                    .align_down(PAGE_SIZE)
                    / PAGE_SIZE;
                log::debug!(
                    "Found usable region, start:{:x}, end:{:x}",
                    regions[i].base(),
                    regions[i].base() + regions[i].len()
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
    fn add_frame(&mut self, _start: usize, _end: usize) {
        warn!("BootstrapFrameAllocator does not need to add frames");
    }

    fn alloc(&mut self, _layout: Layout) -> Option<Paddr> {
        warn!("BootstrapFrameAllocator does not support to allocate memory described by range");
        None
    }

    fn alloc_page(&mut self, _align: usize) -> Option<Paddr> {
        let frame = self.frame_cursor;
        // debug!("allocating frame: {:#x}", frame * PAGE_SIZE,);
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
