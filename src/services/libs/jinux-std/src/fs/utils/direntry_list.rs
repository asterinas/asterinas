use crate::prelude::*;

/// DirEntryList is used to store the entries of a directory.
/// It can guarantee that the index of one entry remains unchanged.
pub struct DirEntryList<T> {
    inner: Vec<Option<T>>,
    len: usize,
}

impl<T> DirEntryList<T> {
    /// New an empty list.
    pub fn new() -> Self {
        Self {
            inner: Vec::new(),
            len: 0,
        }
    }

    /// Returns `true` if the list contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Append an entry to any empty slot of the list or the back of the list.
    pub fn append(&mut self, entry: T) {
        match self.inner.iter().position(|x| x.is_none()) {
            Some(idx) => {
                self.inner[idx] = Some(entry);
            }
            None => {
                self.inner.push(Some(entry));
            }
        }
        self.len += 1;
    }

    /// Removes and returns the entry at position `idx`.
    /// Returns `None` if `idx` is out of bounds or the entry has been removed.
    pub fn remove(&mut self, idx: usize) -> Option<T> {
        if idx >= self.inner.len() {
            return None;
        }
        let mut del_entry = None;
        core::mem::swap(&mut del_entry, &mut self.inner[idx]);
        if del_entry.is_some() {
            debug_assert!(self.len > 0);
            self.len -= 1;
        }
        del_entry
    }

    /// Substitute and returns the entry at position `idx`.
    /// Returns `None` if `idx` is out of bounds or the entry has been removed.
    pub fn substitute(&mut self, idx: usize, entry: T) -> Option<T> {
        if idx >= self.inner.len() {
            return None;
        }
        let mut sub_entry = Some(entry);
        core::mem::swap(&mut sub_entry, &mut self.inner[idx]);
        if sub_entry.is_none() {
            self.len += 1;
        }
        sub_entry
    }

    /// Creates an iterator which gives the index and the entry.
    pub fn enumerate(&self) -> impl Iterator<Item = (usize, &'_ T)> {
        self.inner
            .iter()
            .enumerate()
            .filter(|(_, x)| x.is_some())
            .map(|(idx, x)| (idx, x.as_ref().unwrap()))
    }
}
