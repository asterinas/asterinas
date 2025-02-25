// SPDX-License-Identifier: MPL-2.0

//! A fixed-size local cache for frame allocation.

use core::{alloc::Layout, cell::RefCell};

use ostd::{
    cpu_local, impl_frame_meta_for,
    mm::{
        frame::linked_list::{Link, LinkedList},
        Paddr, UniqueFrame, PAGE_SIZE,
    },
    trap::DisabledLocalIrqGuard,
};

use crate::chunk::{greater_order_of, lesser_order_of, max_order_from, size_of_order, BuddyOrder};

/// The maximum order of a chunk that can be cached.
///
/// So allocations less than 4KiBs*2^3 = 32KiBs will be cached.
const MAX_CACHE_ORDER: BuddyOrder = 4;

/// The expected size of the cache.
const EXPECTED_CACHE_SIZE: usize = PAGE_SIZE * (1 << MAX_CACHE_ORDER);

/// The maximum size of the cache of each order.
const MAX_CACHE_SIZE: usize = EXPECTED_CACHE_SIZE * 2;

cpu_local! {
    static CACHE: RefCell<CacheOfSizes> = RefCell::new(CacheOfSizes::new());
}

struct CacheOfSizes {
    inner: [LinkedList<CachedFreeFrameMeta>; MAX_CACHE_ORDER as usize],
}

impl CacheOfSizes {
    const fn new() -> Self {
        Self {
            inner: [const { LinkedList::new() }; MAX_CACHE_ORDER as usize],
        }
    }

    fn get_mut(&mut self, order: BuddyOrder) -> &mut LinkedList<CachedFreeFrameMeta> {
        &mut self.inner[order as usize]
    }
}

#[derive(Debug)]
struct CachedFreeFrameMeta;

impl_frame_meta_for!(CachedFreeFrameMeta);

pub(super) fn alloc(guard: &DisabledLocalIrqGuard, layout: Layout) -> Option<Paddr> {
    let size_order = greater_order_of(layout.size());
    if size_order >= MAX_CACHE_ORDER {
        return super::pools::alloc(guard, layout);
    }

    let cache_cell = CACHE.get_with(guard);
    let mut cache = cache_cell.borrow_mut();

    if let Some(frame) = cache.get_mut(size_order).pop_front() {
        let addr = frame.start_paddr();
        frame.reset_as_unused();
        Some(addr)
    } else {
        let allocated = super::pools::alloc(
            guard,
            Layout::from_size_align(EXPECTED_CACHE_SIZE, EXPECTED_CACHE_SIZE).unwrap(),
        )?;
        for i in 1..(EXPECTED_CACHE_SIZE / size_of_order(size_order)) {
            let paddr = allocated + (PAGE_SIZE << size_order) * i;
            let frame = UniqueFrame::from_unused(paddr, Link::new(CachedFreeFrameMeta)).unwrap();
            cache.get_mut(size_order).push_front(frame);
        }
        Some(allocated)
    }
}

pub(super) fn dealloc(guard: &DisabledLocalIrqGuard, mut addr: Paddr, mut size: usize) {
    let cache_cell = CACHE.get_with(guard);
    let mut cache = cache_cell.borrow_mut();

    while size > 0 {
        let next_chunk_order = max_order_from(addr).min(lesser_order_of(size));
        let next_chunk_size = size_of_order(next_chunk_order);

        if next_chunk_order >= MAX_CACHE_ORDER {
            super::pools::dealloc(guard, addr, next_chunk_size);
        } else if (cache.get_mut(next_chunk_order).size() + 1) * next_chunk_size >= MAX_CACHE_SIZE {
            super::pools::dealloc(guard, addr, next_chunk_size);

            while cache.get_mut(next_chunk_order).size() * next_chunk_size > EXPECTED_CACHE_SIZE {
                let frame = cache.get_mut(next_chunk_order).pop_back().unwrap();
                let addr = frame.start_paddr();
                frame.reset_as_unused();
                super::pools::dealloc(guard, addr, next_chunk_size);
            }
        } else {
            let frame = UniqueFrame::from_unused(addr, Link::new(CachedFreeFrameMeta)).unwrap();
            cache.get_mut(next_chunk_order).push_front(frame);
        }

        size -= next_chunk_size;
        addr += next_chunk_size;
    }
}
