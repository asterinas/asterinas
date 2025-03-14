// SPDX-License-Identifier: MPL-2.0

//! Slabs for implementing the slab allocator.

use core::{alloc::AllocError, ptr::NonNull};

use super::{slot::HeapSlot, slot_list::SlabSlotList};
use crate::mm::{
    frame::{linked_list::Link, meta::AnyFrameMeta},
    paddr_to_vaddr, FrameAllocOptions, UniqueFrame, PAGE_SIZE,
};

/// A slab.
///
/// The slot size is the maximum size of objects that can be allocated from the
/// slab. The slab is densely divided into slots of this size.
///
/// The `SLOT_SIZE` is the size of the slots in bytes. The size of the slots
/// cannot be smaller than the size of [`usize`]. It must be smaller than or
/// equal to [`PAGE_SIZE`].
///
/// A slab should have the size of one basic page. This restriction may be
/// lifted in the future.
pub type Slab<const SLOT_SIZE: usize> = UniqueFrame<Link<SlabMeta<SLOT_SIZE>>>;

/// Frame metadata of a slab.
///
/// Each slab is backed by a [`UniqueFrame`].
#[derive(Debug)]
pub struct SlabMeta<const SLOT_SIZE: usize> {
    /// The list of free slots inside the slab.
    ///
    /// Slots not inside the slab should not be in the list.
    free_list: SlabSlotList<SLOT_SIZE>,

    /// The number of allocated slots in the slab.
    ///
    /// Even if a slot is free, as long as it does not stay in the
    /// [`Self::free_list`], it is considered allocated.
    nr_allocated: u16,
}

unsafe impl<const SLOT_SIZE: usize> Send for SlabMeta<SLOT_SIZE> {}
unsafe impl<const SLOT_SIZE: usize> Sync for SlabMeta<SLOT_SIZE> {}

unsafe impl<const SLOT_SIZE: usize> AnyFrameMeta for SlabMeta<SLOT_SIZE> {
    fn on_drop(&mut self, _reader: &mut crate::mm::VmReader<crate::mm::Infallible>) {
        if self.nr_allocated != 0 {
            // FIXME: We have no mechanisms to forget the slab once we are here,
            // so we require the user to deallocate all slots before dropping.
            panic!("{} slots allocated when dropping a slab", self.nr_allocated);
        }
    }

    fn is_untyped(&self) -> bool {
        false
    }
}

impl<const SLOT_SIZE: usize> SlabMeta<SLOT_SIZE> {
    /// Gets the capacity of the slab (regardless of the number of allocated slots).
    pub const fn capacity(&self) -> u16 {
        (PAGE_SIZE / SLOT_SIZE) as u16
    }

    /// Gets the number of allocated slots.
    pub fn nr_allocated(&self) -> u16 {
        self.nr_allocated
    }

    /// Allocates a slot from the slab.
    pub fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        let Some(allocated) = self.free_list.pop() else {
            log::error!("Allocating a slot from a full slab");
            return Err(AllocError);
        };
        self.nr_allocated += 1;
        Ok(allocated)
    }
}

impl<const SLOT_SIZE: usize> Slab<SLOT_SIZE> {
    /// Allocates a new slab of the given size.
    ///
    /// If the size is less than `SLOT_SIZE` or [`PAGE_SIZE`], the size will be
    /// the maximum of the two.
    pub fn new() -> crate::prelude::Result<Self> {
        const { assert!(SLOT_SIZE <= PAGE_SIZE) };
        // To ensure we can store a pointer in each slot.
        const { assert!(SLOT_SIZE >= core::mem::size_of::<usize>()) };
        // To ensure `nr_allocated` can be stored in a `u16`.
        const { assert!(PAGE_SIZE / SLOT_SIZE <= u16::MAX as usize) };

        let mut slab: Slab<SLOT_SIZE> = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(Link::new(SlabMeta::<SLOT_SIZE> {
                free_list: SlabSlotList::new(),
                nr_allocated: 0,
            }))?
            .try_into()
            .unwrap();

        let head_paddr = slab.start_paddr();
        let head_vaddr = paddr_to_vaddr(head_paddr);

        // Push each slot to the free list.
        for slot_offset in (0..PAGE_SIZE).step_by(SLOT_SIZE) {
            // SAFETY: The slot is within the slab so it can't be NULL.
            let slot_ptr = unsafe { NonNull::new_unchecked((head_vaddr + slot_offset) as *mut u8) };
            // SAFETY: The slot is newly allocated in the slab.
            slab.meta_mut()
                .free_list
                .push(unsafe { HeapSlot::new(slot_ptr, super::SlotInfo::SlabSlot(SLOT_SIZE)) });
        }

        Ok(slab)
    }

    /// Deallocates a slot to the slab.
    ///
    /// If the slot does not belong to the slab it returns [`AllocError`].
    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if !(self.start_paddr()..self.start_paddr() + self.size()).contains(&slot.paddr()) {
            log::error!("Deallocating a slot to a slab that does not own the slot");
            return Err(AllocError);
        }
        debug_assert_eq!(slot.size(), SLOT_SIZE);
        self.meta_mut().free_list.push(slot);
        self.meta_mut().nr_allocated -= 1;

        Ok(())
    }
}
