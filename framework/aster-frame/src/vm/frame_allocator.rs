// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use alloc::vec::Vec;
use buddy_system_allocator::FrameAllocator;
use log::info;
use spin::Once;

use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};
use crate::{config::PAGE_SIZE, sync::SpinLock};

use super::{frame::VmFrameFlags, VmFrame, VmFrameVec, VmSegment};

pub(super) static FRAME_ALLOCATOR: Once<SpinLock<FrameAllocator>> = Once::new();

pub(crate) fn alloc(nframes: usize, flags: VmFrameFlags) -> Option<VmFrameVec> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .map(|start| {
            let mut vector = Vec::new();
            // Safety: The frame index is valid.
            unsafe {
                for i in 0..nframes {
                    let frame = VmFrame::new(
                        (start + i) * PAGE_SIZE,
                        flags.union(VmFrameFlags::NEED_DEALLOC),
                    );
                    vector.push(frame);
                }
            }
            VmFrameVec(vector)
        })
}

pub(crate) fn alloc_single(flags: VmFrameFlags) -> Option<VmFrame> {
    FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).map(|idx|
            // Safety: The frame index is valid.
            unsafe { VmFrame::new(idx * PAGE_SIZE, flags.union(VmFrameFlags::NEED_DEALLOC)) })
}

pub(crate) fn alloc_contiguous(nframes: usize, flags: VmFrameFlags) -> Option<VmSegment> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes)
        .map(|start|
            // Safety: The range of page frames is contiguous and valid.
            unsafe {
            VmSegment::new(
                start * PAGE_SIZE,
                nframes,
                flags.union(VmFrameFlags::NEED_DEALLOC),
            )
        })
}

/// Deallocate a frame.
///
/// # Safety
///
/// User should ensure the index is valid
///
pub(crate) unsafe fn dealloc_single(index: usize) {
    FRAME_ALLOCATOR.get().unwrap().lock().dealloc(index, 1);
}

/// Deallocate a contiguous range of page frames.
///
/// # Safety
///
/// User should ensure the range of page frames is valid.
///
pub(crate) unsafe fn dealloc_contiguous(start_index: usize, nframes: usize) {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .dealloc(start_index, nframes);
}

pub(crate) fn init(regions: &[MemoryRegion]) {
    let mut allocator = FrameAllocator::<32>::new();
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Usable {
            // Make the memory region page-aligned, and skip if it is too small.
            let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
            let end = (region.base() + region.len()).align_down(PAGE_SIZE) / PAGE_SIZE;
            if end <= start {
                continue;
            }
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
