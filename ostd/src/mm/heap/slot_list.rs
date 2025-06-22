// SPDX-License-Identifier: MPL-2.0

//! Implementation of the free heap slot list.

use core::ptr::NonNull;

use super::HeapSlot;

/// A singly-linked list of [`HeapSlot`]s from [`super::Slab`]s.
///
/// The slots inside this list will have a size of `SLOT_SIZE`. They can come
/// from different slabs.
#[derive(Debug)]
pub struct SlabSlotList<const SLOT_SIZE: usize> {
    /// The head of the list.
    head: Option<NonNull<u8>>,
}

// SAFETY: Any access or modification (i.e., push and pop operations) to the
// data pointed to by `head` requires a `&mut SlabSlotList`. Therefore, at any
// given time, only one task can access the inner `head`. Additionally, a
// `HeapSlot` will not be allocated again as long as it remains in the list.
unsafe impl<const SLOT_SIZE: usize> Sync for SlabSlotList<SLOT_SIZE> {}
unsafe impl<const SLOT_SIZE: usize> Send for SlabSlotList<SLOT_SIZE> {}

impl<const SLOT_SIZE: usize> Default for SlabSlotList<SLOT_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOT_SIZE: usize> SlabSlotList<SLOT_SIZE> {
    /// Creates a new empty list.
    pub const fn new() -> Self {
        Self { head: None }
    }

    /// Pushes a slot to the front of the list.
    ///
    /// # Panics
    ///
    /// Panics if
    ///  - the slot does not come from a slab
    ///    (i.e., `!matches(slot.info(), SlotInfo::SlabSlot(_))`);
    ///  - the size of the slot does not match `SLOT_SIZE`.
    pub fn push(&mut self, slot: HeapSlot) {
        let slot_ptr = slot.as_ptr();
        let super::SlotInfo::SlabSlot(slot_size) = slot.info() else {
            panic!("The slot does not come from a slab");
        };

        assert_eq!(slot_size, SLOT_SIZE);
        const { assert!(SLOT_SIZE >= core::mem::size_of::<usize>()) };

        let original_head = self.head;

        debug_assert!(!slot_ptr.is_null());
        // SAFETY: A pointer to a slot must not be NULL;
        self.head = Some(unsafe { NonNull::new_unchecked(slot_ptr) });
        // Write the original head to the slot.
        // SAFETY: A heap slot must be free so the pointer to the slot can be
        // written to. The slot size is at least the size of a pointer.
        unsafe {
            slot_ptr
                .cast::<usize>()
                .write(original_head.map_or(0, |h| h.as_ptr() as usize));
        }
    }

    /// Pops a slot from the front of the list.
    ///
    /// It returns `None` if the list is empty.
    pub fn pop(&mut self) -> Option<HeapSlot> {
        let original_head = self.head?;

        // SAFETY: The head is a valid pointer to a free slot.
        // The slot contains a pointer to the next slot.
        let next = unsafe { original_head.as_ptr().cast::<usize>().read() } as *mut u8;

        self.head = if next.is_null() {
            None
        } else {
            // SAFETY: We already verified that the next slot is not NULL.
            Some(unsafe { NonNull::new_unchecked(next) })
        };

        Some(unsafe { HeapSlot::new(original_head, super::SlotInfo::SlabSlot(SLOT_SIZE)) })
    }
}
