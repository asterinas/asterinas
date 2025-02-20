// SPDX-License-Identifier: MPL-2.0

use core::alloc::Layout;

use super::{Heap, *};
use crate::{mm::PAGE_SIZE, prelude::*};

#[ktest]
fn heap_initialization() {
    unsafe {
        init();

        assert!(HEAP_ALLOCATOR.heap.get().is_some());
    }
}

#[ktest]
fn heap_allocator_new() {
    let locked_heap = LockedHeapWithRescue::new();
    assert!(locked_heap.heap.get().is_none());
}

#[ktest]
fn heap_allocator_alloc() {
    unsafe {
        init();

        let layout = Layout::from_size_align(16, 8).unwrap();
        let ptr = HEAP_ALLOCATOR.alloc(layout);

        assert!(!ptr.is_null());
        HEAP_ALLOCATOR.dealloc(ptr, layout);
    }
}

#[ktest]
fn heap_allocator_dealloc() {
    unsafe {
        init();

        let layout = Layout::from_size_align(16, 8).unwrap();
        let ptr = HEAP_ALLOCATOR.alloc(layout);

        HEAP_ALLOCATOR.dealloc(ptr, layout);

        let ptr2 = HEAP_ALLOCATOR.alloc(layout);
        assert!(!ptr2.is_null());

        HEAP_ALLOCATOR.dealloc(ptr2, layout);
    }
}

#[ktest]
fn heap_allocator_large_layout() {
    unsafe {
        init();

        let layout = Layout::from_size_align(PAGE_SIZE * 1024, PAGE_SIZE).unwrap();
        let ptr = HEAP_ALLOCATOR.alloc(layout);

        assert!(!ptr.is_null());

        HEAP_ALLOCATOR.dealloc(ptr, layout);
    }
}

#[ktest]
fn heap_stat() {
    #[repr(align(4096))]
    struct MockHeapSpace([u8; PAGE_SIZE * 8]);

    unsafe {
        let mut buffer = MockHeapSpace([0u8; PAGE_SIZE * 8]);
        let heap = Heap::new(buffer.0.as_mut_ptr() as usize, PAGE_SIZE * 8);

        let layout: Layout = Layout::from_size_align(16, 8).unwrap();
        let size = heap.usable_size(layout);
        assert_eq!(size.0, 16);

        let total_bytes = heap.total_bytes();
        assert_eq!(total_bytes, PAGE_SIZE * 8);

        let used_bytes = heap.used_bytes();
        assert_eq!(used_bytes, 0);

        let available_bytes = heap.available_bytes();
        assert_eq!(available_bytes, PAGE_SIZE * 8);
    }
}
