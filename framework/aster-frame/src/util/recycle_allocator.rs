// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

#[derive(Debug)]
pub struct RecycleAllocator {
    current: usize,
    recycled: Vec<usize>,
    skip: Vec<usize>,
    max: usize,
}

impl RecycleAllocator {
    pub const fn new() -> Self {
        RecycleAllocator {
            current: 0,
            recycled: Vec::new(),
            skip: Vec::new(),
            max: usize::MAX - 1,
        }
    }

    pub const fn with_start_max(start: usize, max: usize) -> Self {
        RecycleAllocator {
            current: start,
            recycled: Vec::new(),
            skip: Vec::new(),
            max,
        }
    }

    #[allow(unused)]
    pub fn alloc(&mut self) -> usize {
        if let Some(id) = self.recycled.pop() {
            return id;
        }
        // recycle list is empty, need to use current to allocate an id.
        // it should skip the element in skip list
        while self.skip.contains(&self.current) {
            self.current += 1;
        }
        if self.current == self.max {
            return usize::MAX;
        }
        self.current += 1;
        self.current - 1
    }
    /// deallocate a id, it should fit one of the following requirement, otherwise it will panic:
    ///
    /// 1. It is in the skip list
    ///
    /// 2. It smaller than current and not in recycled list
    #[allow(unused)]
    pub fn dealloc(&mut self, id: usize) {
        if !self.skip.contains(&id) {
            assert!(id < self.current);
            assert!(
                !self.recycled.iter().any(|i| *i == id),
                "id {} has been deallocated!",
                id
            );
        } else {
            // if the value is in skip list, then remove it from the skip list
            self.skip.retain(|value| *value != id);
        }
        self.recycled.push(id);
    }

    /// get target id in the list, it will return true if the target can used, false if can not used.
    /// the target need to meet one of the following requirement so that it can used:
    ///
    /// 1. It is in the recycled list
    ///
    /// 2. It is bigger than the current, smaller than max and not in the skip list
    ///
    pub fn get_target(&mut self, target: usize) -> bool {
        if target >= self.max {
            return false;
        }
        if target >= self.current {
            if self.skip.contains(&target) {
                false
            } else {
                self.skip.push(target);
                true
            }
        } else if self.recycled.contains(&target) {
            self.recycled.retain(|value| *value != target);
            true
        } else {
            false
        }
    }
}
