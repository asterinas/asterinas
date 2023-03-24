use crate::prelude::*;

/// DirEntryVec is used to store the entries of a directory.
/// It can guarantee that the index of one dir entry remains unchanged during
/// adding or deleting other dir entries of it.
pub struct DirEntryVec<T> {
    // The slots to store dir entries.
    slots: Vec<Option<T>>,
    // The number of occupied slots.
    // The i-th slot is occupied if `self.slots[i].is_some()`.
    num_occupied: usize,
}

impl<T> DirEntryVec<T> {
    /// New an empty vec.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            num_occupied: 0,
        }
    }

    /// Returns `true` if the vec contains no entries.
    pub fn is_empty(&self) -> bool {
        self.num_occupied == 0
    }

    /// Put a dir entry into the vec.
    /// it may be put into an existing empty slot or the back of the vec.
    pub fn put(&mut self, entry: T) {
        if self.num_occupied == self.slots.len() {
            self.slots.push(Some(entry));
        } else {
            let idx = self.slots.iter().position(|x| x.is_none()).unwrap();
            self.slots[idx] = Some(entry);
        }
        self.num_occupied += 1;
    }

    /// Removes and returns the entry at position `idx`.
    /// Returns `None` if `idx` is out of bounds or the entry has been removed.
    pub fn remove(&mut self, idx: usize) -> Option<T> {
        if idx >= self.slots.len() {
            return None;
        }
        let mut del_entry = None;
        core::mem::swap(&mut del_entry, &mut self.slots[idx]);
        if del_entry.is_some() {
            debug_assert!(self.num_occupied > 0);
            self.num_occupied -= 1;
        }
        del_entry
    }

    /// Put and returns the entry at position `idx`.
    /// Returns `None` if `idx` is out of bounds or the entry has been removed.
    pub fn put_at(&mut self, idx: usize, entry: T) -> Option<T> {
        if idx >= self.slots.len() {
            return None;
        }
        let mut sub_entry = Some(entry);
        core::mem::swap(&mut sub_entry, &mut self.slots[idx]);
        if sub_entry.is_none() {
            self.num_occupied += 1;
        }
        sub_entry
    }

    /// Creates an iterator which gives both of the index and the dir entry.
    /// The index may not be continuous.
    pub fn idxes_and_entries(&self) -> impl Iterator<Item = (usize, &'_ T)> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, x)| x.is_some())
            .map(|(idx, x)| (idx, x.as_ref().unwrap()))
    }

    /// Creates an iterator which gives the dir entry.
    pub fn iter(&self) -> impl Iterator<Item = &'_ T> {
        self.slots.iter().filter_map(|x| x.as_ref())
    }
}
