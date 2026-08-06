// SPDX-License-Identifier: MPL-2.0

//! Tests for the kernel slab allocation counter.

use alloc::vec::Vec;

use ostd::{
    mm::{PAGE_SIZE, heap::HeapSlot},
    prelude::ktest,
};

use crate::slab_cache::SlabCache;

/// Allocating enough slots forces new slabs, increasing the counter by at
/// least one page each, and freeing them releases some slabs back.
#[ktest]
fn slab_counter_tracks_slab_pages() {
    const NR_SLABS: usize = 20;
    let before = crate::load_total_slab_size();

    let mut cache = SlabCache::<8>::new();
    let mut slots: Vec<HeapSlot> = Vec::with_capacity(NR_SLABS * (PAGE_SIZE / 8));
    for _ in 0..NR_SLABS * (PAGE_SIZE / 8) {
        slots.push(cache.alloc().unwrap());
    }

    let after_alloc = crate::load_total_slab_size();
    assert!(
        after_alloc >= before + NR_SLABS * PAGE_SIZE,
        "allocating slots should create at least {NR_SLABS} slabs, \
         before={before}, after={after_alloc}",
    );

    for slot in slots {
        cache.dealloc(slot).unwrap();
    }

    let after_free = crate::load_total_slab_size();
    assert!(
        after_free < after_alloc,
        "freeing all slots should release some slabs, \
         before_free={after_alloc}, after_free={after_free}",
    );

    // Dropping the cache releases the remaining slabs back to the frame
    // allocator, so the counter returns to its original value.
    drop(cache);
    let after_drop = crate::load_total_slab_size();
    assert_eq!(
        after_drop, before,
        "dropping the cache should return the counter to its original value, \
         expected={before}, actual={after_drop}",
    );
}
