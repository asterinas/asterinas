// SPDX-License-Identifier: MPL-2.0

//! Implementation of the free heap slot list.

use core::ptr::NonNull;

use super::HeapSlot;

/// A singly list of free heap slots [`HeapSlot`].
///
/// The slots inside this list will not be larger than `SLOT_SIZE`.
#[derive(Debug)]
pub struct HeapSlotList<const SLOT_SIZE: usize> {
    /// The head of the list.
    head: Option<NonNull<u8>>,
}

impl<const SLOT_SIZE: usize> Default for HeapSlotList<SLOT_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOT_SIZE: usize> HeapSlotList<SLOT_SIZE> {
    /// Creates a new empty list.
    pub const fn new() -> Self {
        Self { head: None }
    }

    /// Pushes a slot to the front of the list.
    ///
    /// # Panics
    ///
    /// Panics if the size of the slot is larger than `SLOT_SIZE`.
    pub fn push(&mut self, slot: HeapSlot) {
        let slot_ptr = slot.as_ptr();
        let slot_size = slot.size();

        assert!(slot_size <= SLOT_SIZE);
        debug_assert!(SLOT_SIZE >= core::mem::size_of::<usize>());

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

        Some(unsafe { HeapSlot::new(original_head, SLOT_SIZE) })
    }
}
