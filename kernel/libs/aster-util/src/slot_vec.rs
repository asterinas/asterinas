// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

/// A `SlotVec` is a type of vector that retains the index of an item, even when items are added to
/// or removed from the vector.
#[derive(Clone, Debug)]
pub struct SlotVec<T> {
    /// The slots to store items.
    slots: Vec<Option<T>>,
    /// The number of occupied slots.
    ///
    /// The `i`-th slot is occupied if `self.slots[i].is_some()`.
    num_occupied: usize,
}

impl<T> SlotVec<T> {
    /// Creates a new, empty vector.
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            num_occupied: 0,
        }
    }

    /// Creates an empty vector with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            num_occupied: 0,
        }
    }

    /// Returns `true` if the vector contains no items.
    pub fn is_empty(&self) -> bool {
        self.num_occupied == 0
    }

    /// Returns the number of items.
    pub fn len(&self) -> usize {
        self.num_occupied
    }

    /// Returns the number of slots.
    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    /// Gets the item at position `idx`.
    ///
    /// Returns `None` if `idx` is out of bounds or the item does not exist.
    pub fn get(&self, idx: usize) -> Option<&T> {
        self.slots.get(idx)?.as_ref()
    }

    /// Gets a mutable reference to the item at position `idx`.
    ///
    /// Returns `None` if `idx` is out of bounds or the item does not exist.
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        self.slots.get_mut(idx)?.as_mut()
    }

    /// Puts an item into the vector.
    ///
    /// It may be put in an empty slot or at the back of the vector.
    ///
    /// Returns the index of the inserted item.
    pub fn put(&mut self, entry: T) -> usize {
        let idx = if self.num_occupied == self.slots.len() {
            self.slots.push(Some(entry));
            self.slots.len() - 1
        } else {
            let idx = self.slots.iter().position(|x| x.is_none()).unwrap();
            self.slots[idx] = Some(entry);
            idx
        };
        self.num_occupied += 1;
        idx
    }

    /// Puts and returns the item at position `idx`.
    ///
    /// Returns `None` if the item does not exist.
    pub fn put_at(&mut self, idx: usize, item: T) -> Option<T> {
        if idx >= self.slots.len() {
            self.slots.resize_with(idx + 1, Default::default);
        }
        let mut sub_item = Some(item);
        core::mem::swap(&mut sub_item, &mut self.slots[idx]);
        if sub_item.is_none() {
            self.num_occupied += 1;
        }
        sub_item
    }

    /// Removes and returns the item at position `idx`.
    ///
    /// Returns `None` if `idx` is out of bounds or the item does not exist.
    pub fn remove(&mut self, idx: usize) -> Option<T> {
        if idx >= self.slots.len() {
            return None;
        }
        let mut del_item = None;
        core::mem::swap(&mut del_item, &mut self.slots[idx]);
        if del_item.is_some() {
            debug_assert!(self.num_occupied > 0);
            self.num_occupied -= 1;
        }
        del_item
    }

    /// Creates an iterator which gives both the index and the item.
    ///
    /// The indices may not be continuous.
    pub fn idxes_and_items(&self) -> impl Iterator<Item = (usize, &'_ T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.as_ref().map(|val| (idx, val)))
    }

    /// Creates an iterator which gives only the item.
    pub fn iter(&self) -> impl Iterator<Item = &'_ T> {
        self.slots.iter().filter_map(|x| x.as_ref())
    }
}

impl Default for SlotVec<()> {
    fn default() -> Self {
        Self::new()
    }
}
