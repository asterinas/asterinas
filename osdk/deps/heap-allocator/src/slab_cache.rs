// SPDX-License-Identifier: MPL-2.0

//! The slab cache that is composed of slabs.

use core::alloc::AllocError;

use ostd::mm::{
    frame::linked_list::LinkedList,
    heap::{HeapSlot, Slab, SlabMeta},
    Paddr, PAGE_SIZE,
};

const EXPECTED_EMPTY_SLABS: usize = 4;
const MAX_EMPTY_SLABS: usize = 16;

/// A slab cache.
///
/// A slab cache contains 3 parts:
///  - a list of empty slabs;
///  - a list of partially allocated slabs;
///  - and a list of full slabs.
///
/// So the cache is partially sorted, to allow caching and reusing memory.
pub struct SlabCache<const SLOT_SIZE: usize> {
    empty: LinkedList<SlabMeta<SLOT_SIZE>>,
    partial: LinkedList<SlabMeta<SLOT_SIZE>>,
    full: LinkedList<SlabMeta<SLOT_SIZE>>,
}

impl<const SLOT_SIZE: usize> SlabCache<SLOT_SIZE> {
    /// Creates a new slab cache.
    pub const fn new() -> Self {
        Self {
            empty: LinkedList::new(),
            partial: LinkedList::new(),
            full: LinkedList::new(),
        }
    }

    /// Allocates a slot from the cache.
    ///
    /// The caller must provide which cache is it because we don't know from
    /// `&mut self`. The information is used for deallocation.
    pub fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        // Try to allocate from the partial slabs first.
        if !self.partial.is_empty() {
            let mut cursor = self.partial.cursor_back_mut();
            let current = cursor.current_meta().unwrap();
            let allocated = current.alloc().unwrap();
            if current.nr_allocated() == current.capacity() {
                self.full.push_front(cursor.take_current().unwrap());
            }
            return Ok(allocated);
        }

        // If no partial slab is available, try to get an empty slab.
        if !self.empty.is_empty() {
            let mut slab = self.empty.pop_front().unwrap();
            let allocated = slab.meta_mut().alloc().unwrap();
            self.add_slab(slab);
            return Ok(allocated);
        }

        // If no empty slab is available, allocate new slabs.
        let Ok(mut allocated_empty) = Slab::new() else {
            log::error!("Failed to allocate a new slab");
            return Err(AllocError);
        };
        let allocated = allocated_empty.meta_mut().alloc().unwrap();
        self.add_slab(allocated_empty);

        // Allocate more empty slabs and push them into the cache.
        for _ in 0..EXPECTED_EMPTY_SLABS {
            if let Ok(allocated_empty) = Slab::new() {
                self.empty.push_front(allocated_empty);
            } else {
                break;
            }
        }

        Ok(allocated)
    }

    /// Deallocates a slot into the cache.
    ///
    /// The slot must be allocated from the cache.
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        let which = which_slab(&slot).ok_or_else(|| {
            log::error!("Can't find the slab for the slot");
            AllocError
        })?;

        let mut extracted_slab = None;

        if self.partial.contains(which) {
            extracted_slab = self.partial.cursor_mut_at(which).unwrap().take_current();
        } else if self.full.contains(which) {
            extracted_slab = self.full.cursor_mut_at(which).unwrap().take_current();
        }

        let mut slab = extracted_slab.ok_or_else(|| {
            log::error!("Deallocating a slot that is not allocated from the cache");
            AllocError
        })?;

        slab.dealloc(slot)?;

        self.add_slab(slab);

        // If the slab cache has too many empty slabs, free some of them.
        if self.empty.size() > MAX_EMPTY_SLABS {
            while self.empty.size() > EXPECTED_EMPTY_SLABS {
                self.empty.pop_front();
            }
        }

        Ok(())
    }

    fn add_slab(&mut self, slab: Slab<SLOT_SIZE>) {
        if slab.meta().nr_allocated() == slab.meta().capacity() {
            self.full.push_front(slab);
        } else if slab.meta().nr_allocated() > 0 {
            self.partial.push_back(slab);
        } else {
            self.empty.push_front(slab);
        }
    }
}

/// Gets which slab the slot belongs to.
///
/// If the slot size is larger than [`PAGE_SIZE`], it is not from a slab
/// and this function will return `None`.
///
/// `SLOT_SIZE` can be larger than `slot.size()` but not smaller.
fn which_slab(slot: &HeapSlot) -> Option<Paddr> {
    if slot.size() > PAGE_SIZE {
        return None;
    }

    let frame_paddr = slot.paddr() / PAGE_SIZE * PAGE_SIZE;
    Some(frame_paddr)
}
