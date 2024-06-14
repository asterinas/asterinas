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

use super::{
    meta::{FrameMeta, PageMeta},
    Page,
};
use crate::{
    boot::memory_region::MemoryRegionType,
    mm::{Frame, FrameVec, Segment, PAGE_SIZE},
    sync::SpinLock,
};

pub(in crate::mm) static FRAME_ALLOCATOR: Once<SpinLock<FrameAllocator>> = Once::new();

pub(crate) fn alloc(nframes: usize) -> Option<FrameVec> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .map(|start| {
            let mut vector = Vec::new();
            for i in 0..nframes {
                let paddr = (start + i) * PAGE_SIZE;
                let frame = Frame {
                    page: Page::<FrameMeta>::from_unused(paddr),
                };
                vector.push(frame);
            }
            FrameVec(vector)
        })
}

pub(crate) fn alloc_single<T: PageMeta>() -> Option<Page<T>> {
    FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).map(|idx| {
        let paddr = idx * PAGE_SIZE;
        Page::<T>::from_unused(paddr)
    })
}

pub(crate) fn alloc_contiguous(nframes: usize) -> Option<Segment> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .map(|start|
            // SAFETY: The range of page frames is contiguous and valid.
            unsafe {
            Segment::new(
                start * PAGE_SIZE,
                nframes,
            )
        })
}

/// Deallocates a contiguous range of page frames.
///
/// # Safety
///
/// User should ensure the range of page frames is valid.
///
pub(crate) unsafe fn dealloc(start_index: usize, nframes: usize) {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .dealloc(start_index, nframes);
}

pub(crate) fn init() {
    let regions = crate::boot::memory_regions();
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
            info!(
                "Found usable region, start:{:x}, end:{:x}",
                region.base(),
                region.base() + region.len()
            );
        }
    }
    FRAME_ALLOCATOR.call_once(|| SpinLock::new(allocator));
}
