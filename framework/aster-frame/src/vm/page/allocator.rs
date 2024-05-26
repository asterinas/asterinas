// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use align_ext::AlignExt;
use buddy_system_allocator::FrameAllocator;
use log::info;
use spin::Once;

use super::{meta::FrameMeta, Page, VmFrame, VmFrameVec, VmSegment};
use crate::{boot::memory_region::MemoryRegionType, sync::SpinLock, vm::PAGE_SIZE};

pub(in crate::vm) static FRAME_ALLOCATOR: Once<SpinLock<FrameAllocator>> = Once::new();

pub(crate) fn alloc(nframes: usize) -> Option<VmFrameVec> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .map(|start| {
            let mut vector = Vec::new();
            for i in 0..nframes {
                let paddr = (start + i) * PAGE_SIZE;
                // SAFETY: The frame index is valid.
                let frame = VmFrame {
                    page: Page::<FrameMeta>::from_unused(paddr).unwrap(),
                };
                vector.push(frame);
            }
            VmFrameVec(vector)
        })
}

pub(crate) fn alloc_single() -> Option<VmFrame> {
    FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).map(|idx| {
        let paddr = idx * PAGE_SIZE;
        VmFrame {
            page: Page::<FrameMeta>::from_unused(paddr).unwrap(),
        }
    })
}

pub(crate) fn alloc_contiguous(nframes: usize) -> Option<VmSegment> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .map(|start|
            // SAFETY: The range of page frames is contiguous and valid.
            unsafe {
            VmSegment::new(
                start * PAGE_SIZE,
                nframes,
            )
        })
}

/// Deallocate a contiguous range of page frames.
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
