use align_ext::AlignExt;
use alloc::vec::Vec;
use buddy_system_allocator::FrameAllocator;
use log::info;
use spin::Once;

use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};
use crate::{config::PAGE_SIZE, sync::SpinLock};

use super::{frame::VmFrameFlags, VmFrame};

pub(super) static FRAME_ALLOCATOR: Once<SpinLock<FrameAllocator>> = Once::new();

pub(crate) fn alloc(flags: VmFrameFlags) -> Option<VmFrame> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(1)
        .map(|pa| unsafe { VmFrame::new(pa * PAGE_SIZE, flags.union(VmFrameFlags::NEED_DEALLOC)) })
}

pub(crate) fn alloc_continuous(frame_count: usize, flags: VmFrameFlags) -> Option<Vec<VmFrame>> {
    FRAME_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(frame_count)
        .map(|start| {
            let mut vector = Vec::new();
            unsafe {
                for i in 0..frame_count {
                    let frame = VmFrame::new(
                        (start + i) * PAGE_SIZE,
                        flags.union(VmFrameFlags::NEED_DEALLOC),
                    );
                    vector.push(frame);
                }
            }
            vector
        })
}

pub(crate) fn alloc_zero(flags: VmFrameFlags) -> Option<VmFrame> {
    let frame = alloc(flags)?;
    frame.zero();
    Some(frame)
}

/// Dealloc a frame.
///
/// # Safety
///
/// User should ensure the index is valid
///
pub(crate) unsafe fn dealloc(index: usize) {
    FRAME_ALLOCATOR.get().unwrap().lock().dealloc(index, 1);
}

pub(crate) fn init(regions: &[MemoryRegion]) {
    let mut allocator = FrameAllocator::<32>::new();
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Usable {
            // Make the memory region page-aligned
            let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
            let end = (region.base() + region.len()).align_down(PAGE_SIZE) / PAGE_SIZE;
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
