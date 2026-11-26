// SPDX-License-Identifier: MPL-2.0

//! Providing test utilities and high-level test cases for the frame allocator.

use core::{
    alloc::Layout,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use ostd::{
    mm::{
        FrameAllocOptions, HasPaddr, PAGE_SIZE, Paddr, Segment, UniqueFrame,
        frame::GlobalFrameAllocator,
    },
    prelude::ktest,
};

use super::FrameAllocator;
use crate::{chunk::BuddyOrder, set::BuddySet};

#[ktest]
fn frame_allocator_alloc_layout_match() {
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 3, PAGE_SIZE).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 4, PAGE_SIZE).unwrap());

    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE * 2).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 4, PAGE_SIZE * 4).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 8, PAGE_SIZE * 8).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 16, PAGE_SIZE * 16).unwrap());

    split_allocations();

    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 2, PAGE_SIZE * 2).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 4, PAGE_SIZE * 4).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 8, PAGE_SIZE * 8).unwrap());
    assert_allocation_well_formed(Layout::from_size_align(PAGE_SIZE * 16, PAGE_SIZE * 16).unwrap());
}

#[track_caller]
fn assert_allocation_well_formed(layout: Layout) {
    let instance = FrameAllocator;

    let allocated = instance.alloc(layout).unwrap();
    assert_eq!(
        allocated % layout.align(),
        0,
        "Allocation alignment mismatch"
    );

    for offset in (0..layout.size()).step_by(PAGE_SIZE) {
        let frame = allocated + offset;
        let frame = UniqueFrame::from_unused(frame, ()).unwrap_or_else(|e| {
            panic!(
                "Metadata ill-formed after allocation at offset {:#x}: {:#?}",
                offset, e
            )
        });
        frame.reset_as_unused();
    }

    instance.add_free_memory(allocated, layout.size());
}

fn split_allocations() {
    let instance = FrameAllocator;

    // Allocate 16 frames.
    let layout = Layout::from_size_align(PAGE_SIZE * 16, PAGE_SIZE).unwrap();
    let allocated = instance.alloc(layout).unwrap();

    // Deallocate 16 frames.
    // |-|--|----|--------|-|
    //  1 2   4      8     1
    instance.dealloc(allocated, PAGE_SIZE);
    instance.dealloc(allocated + PAGE_SIZE, PAGE_SIZE * 2);
    instance.dealloc(allocated + PAGE_SIZE * 3, PAGE_SIZE * 4);
    instance.dealloc(allocated + PAGE_SIZE * 7, PAGE_SIZE * 8);
    instance.dealloc(allocated + PAGE_SIZE * 15, PAGE_SIZE);
}

/// A mocked memory region for testing.
///
/// All the frames in the returned memory region will be marked as used.
/// When the region is dropped, all the frames will be returned to the global
/// frame allocator. If any frame is not unused by that time, the drop will panic.
pub(crate) struct MockMemoryRegion {
    addr: Paddr,
    size: usize,
}

impl MockMemoryRegion {
    /// Gets a memory region for testing.
    pub(crate) fn alloc(size: usize) -> Self {
        let seg = FrameAllocOptions::new()
            .alloc_segment(size / PAGE_SIZE)
            .unwrap();
        let addr = seg.paddr();
        for frame in seg {
            UniqueFrame::try_from(frame).unwrap().reset_as_unused();
        }
        Self { addr, size }
    }

    /// Gets the start address of the memory region.
    pub(crate) fn paddr(&self) -> Paddr {
        self.addr
    }
}

impl Drop for MockMemoryRegion {
    fn drop(&mut self) {
        let seg = Segment::from_unused(self.addr..self.addr + self.size, |_| ()).unwrap();
        drop(seg);
    }
}

/// A [`BuddySet`] wrapper for tests that resets remaining free chunks on drop.
///
/// Borrowing [`MockMemoryRegion`] ensures this wrapper drains `FreeHeadMeta`
/// frames before the backing region reclaims them, including on panic.
pub(crate) struct TestBuddySet<'a, const MAX_ORDER: BuddyOrder> {
    set: BuddySet<MAX_ORDER>,
    _region: PhantomData<&'a MockMemoryRegion>,
}

impl<'a, const MAX_ORDER: BuddyOrder> TestBuddySet<'a, MAX_ORDER> {
    pub(crate) fn new_empty(_region: &'a MockMemoryRegion) -> Self {
        Self {
            set: BuddySet::new_empty(),
            _region: PhantomData,
        }
    }
}

impl<const MAX_ORDER: BuddyOrder> Deref for TestBuddySet<'_, MAX_ORDER> {
    type Target = BuddySet<MAX_ORDER>;

    fn deref(&self) -> &Self::Target {
        &self.set
    }
}

impl<const MAX_ORDER: BuddyOrder> DerefMut for TestBuddySet<'_, MAX_ORDER> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.set
    }
}

impl<const MAX_ORDER: BuddyOrder> Drop for TestBuddySet<'_, MAX_ORDER> {
    fn drop(&mut self) {
        while self.set.alloc_chunk(0).is_some() {}
    }
}
