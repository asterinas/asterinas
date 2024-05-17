


// SPDX-License-Identifier: MPL-2.0

// use alloc::vec::Vec;

use align_ext::AlignExt;
use buddy_system_allocator::FrameAllocator;
// use log::info;
use spin::Once;
// use x86_64::registers::model_specific::Star;

// use super::{frame::VmFrameFlags, VmFrame, VmFrameVec, VmSegment};
use super::kvmar::Kvmar;
use crate::{
    sync::SpinLock,
    vm::PAGE_SIZE,
    arch::mm::PageTableFlags,
};

pub(super) static KVMAR_ALLOCATOR: Once<SpinLock<FrameAllocator>> = Once::new();

// static mut HEAP: [u64]
// nframs一定
pub(crate) fn alloc_contiguous(nframes: usize, flags: PageTableFlags) -> Option<Kvmar> {
    KVMAR_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc(nframes+1)
        .map(|start|
            // Safety: The range of page frames is contiguous and valid.
            Kvmar::new_with_commit_pages(
                start * PAGE_SIZE,
                (nframes+1) * PAGE_SIZE,
                // 这个flags可能需要修改
                flags,
            ).unwrap()
        )
}

pub(crate) fn dealloc_contiguous(start_index: usize, nframes: usize) {
    KVMAR_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .dealloc(start_index, nframes+1);
}
// 将 0xffff ffff C000 0000
pub(crate) fn init(start_vaddr:usize, end_vaddr:usize) {
    let mut allocator = FrameAllocator::<32>::new();
    let start = start_vaddr.align_up(PAGE_SIZE) / PAGE_SIZE;
    let end = end_vaddr.align_down(PAGE_SIZE) / PAGE_SIZE - 1;
    allocator.add_frame(start, end);
    KVMAR_ALLOCATOR.call_once(|| SpinLock::new(allocator));
}
