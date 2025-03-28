// SPDX-License-Identifier: MPL-2.0

//! A pool of zeroed page tables.
//!
//! This is to avoid zeroing pages/doing allocations while holding locks.

use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{
    node::{MapTrackingStatus, PageTablePageMeta},
    PageTableEntryTrait, PageTableLock, PageTableNode,
};
use crate::{
    cpu::PinCurrentCpu,
    cpu_local,
    mm::{FrameAllocOptions, PagingConstsTrait, PagingLevel, UniqueFrame},
    task::DisabledPreemptGuard,
};

const MAX_POOL_SIZE: usize = 12;
const PREFILL_SIZE: usize = 4;

cpu_local! {
    static ZEROED_PT_POOL: RefCell<[Option<UniqueFrame<()>>; MAX_POOL_SIZE]> = RefCell::new([const { None }; MAX_POOL_SIZE]);
    static POOL_SIZE: AtomicUsize = AtomicUsize::new(0);
}

pub(super) fn prefill(preempt_guard: &DisabledPreemptGuard) {
    let cpu = preempt_guard.current_cpu();
    let pool_size = POOL_SIZE.get_on_cpu(cpu);

    let size = pool_size.load(Ordering::Relaxed);

    if size <= PREFILL_SIZE {
        let irq_guard = crate::trap::disable_local();
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

pub(super) fn alloc<E: PageTableEntryTrait, C: PagingConstsTrait>(
    preempt_guard: &DisabledPreemptGuard,
    level: PagingLevel,
    is_tracked: MapTrackingStatus,
) -> PageTableLock<E, C> {
    let cpu = preempt_guard.current_cpu();
    let pool_size = POOL_SIZE.get_on_cpu(cpu);

    let size = pool_size.load(Ordering::Relaxed);

    if size == 0 {
        return PageTableLock::alloc(level, is_tracked);
    }

    let irq_guard = crate::trap::disable_local();
    let pool_deref_guard = ZEROED_PT_POOL.get_with(&irq_guard);
    let mut pool = pool_deref_guard.borrow_mut();
    let frame = pool[size - 1].take().unwrap();
    pool_size.store(size - 1, Ordering::Relaxed);

    let frame: PageTableNode<E, C> = frame
        .repurpose(PageTablePageMeta::<E, C>::new_locked(level, is_tracked))
        .into();

    // SAFETY: The metadata must match the locked frame.
    unsafe { PageTableLock::from_raw_paddr(frame.into_raw()) }
}
