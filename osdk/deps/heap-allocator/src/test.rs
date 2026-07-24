// SPDX-License-Identifier: MPL-2.0

//! Tests for the kernel heap allocation counter.

use core::alloc::Layout;

use ostd::{
    mm::{PAGE_SIZE, heap::GlobalHeapAllocator},
    prelude::ktest,
};

use super::HeapAllocator;

/// Large allocation (direct frame allocation) increases the counter by at
/// least the allocated size, and freeing decreases it accordingly.
#[ktest]
fn kernel_heap_counter_large_alloc() {
    let before = crate::load_total_heap_size();
    let alloc_size = PAGE_SIZE * 4;

    let layout = Layout::from_size_align(alloc_size, PAGE_SIZE).unwrap();
    let slot = HeapAllocator.alloc(layout).unwrap();
    let after_alloc = crate::load_total_heap_size();

    assert!(
        after_alloc >= before + alloc_size,
        "Large alloc should increase counter by at least {alloc_size}, \
         before={before}, after={after_alloc}",
    );

    HeapAllocator.dealloc(slot).unwrap();
    let after_free = crate::load_total_heap_size();

    assert!(
        after_free <= after_alloc - alloc_size,
        "Free should decrease counter by at least {alloc_size}, \
         before_free={after_alloc}, after_free={after_free}",
    );
}
