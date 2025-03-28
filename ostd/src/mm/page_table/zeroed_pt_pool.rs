// SPDX-License-Identifier: MPL-2.0

//! A pool of zeroed page tables.
//!
//! This is to avoid zeroing pages/doing allocations while holding locks.

use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{node::PageTablePageMeta, PageTableConfig, PageTableNode};
use crate::{
    cpu::PinCurrentCpu,
    cpu_local,
    mm::{FrameAllocOptions, PagingLevel, UniqueFrame},
    task::disable_preempt,
    trap::irq,
};

const MAX_POOL_SIZE: usize = 12;
const PREFILL_SIZE: usize = 4;

cpu_local! {
    static ZEROED_PT_POOL: RefCell<[Option<UniqueFrame<()>>; MAX_POOL_SIZE]> = RefCell::new([const { None }; MAX_POOL_SIZE]);
    static POOL_SIZE: AtomicUsize = AtomicUsize::new(0);
}

pub(super) fn prefill() {
    let preempt_guard = disable_preempt();
    let cpu = preempt_guard.current_cpu();
    let pool_size = POOL_SIZE.get_on_cpu(cpu);

    let size = pool_size.load(Ordering::Relaxed);

    if size <= PREFILL_SIZE {
        let irq_guard = irq::disable_local();
        let pool_deref_guard = ZEROED_PT_POOL.get_with(&irq_guard);
        let mut pool = pool_deref_guard.borrow_mut();
        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(MAX_POOL_SIZE - size)
            .unwrap();
        for (i, frame) in segment.enumerate() {
            pool[size + i] = Some(frame.try_into().unwrap());
        }
        pool_size.store(MAX_POOL_SIZE, Ordering::Relaxed);
    }
}

pub(super) fn alloc<C: PageTableConfig>(level: PagingLevel) -> PageTableNode<C> {
    let preempt_guard = disable_preempt();
    let cpu = preempt_guard.current_cpu();
    let pool_size = POOL_SIZE.get_on_cpu(cpu);

    let size = pool_size.load(Ordering::Relaxed);

    let meta = PageTablePageMeta::<C>::new(level);

    if size == 0 {
        return FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)
            .expect("Failed to allocate a page table node");
    }

    let irq_guard = irq::disable_local();
    let pool_deref_guard = ZEROED_PT_POOL.get_with(&irq_guard);
    let mut pool = pool_deref_guard.borrow_mut();
    let frame = pool[size - 1].take().unwrap();
    pool_size.store(size - 1, Ordering::Relaxed);

    frame.repurpose(meta).into()
}
