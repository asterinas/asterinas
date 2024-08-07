// SPDX-License-Identifier: MPL-2.0 OR MIT

// To be specific, the original source code is from
// [buddy_system_allocator](https://github.com/rcore-os/buddy_system_allocator),
// which licensed under the following license.
//
// SPDX-License-Identifier: MIT
//
// Copyright (c) 2019-2020 Jiajie Chen
//
// We make the following new changes:
// * Add `alloc_specific` to allocate a specific range of frames.
// * Implement `PageAlloc` trait for `BuddyFrameAllocator`.
//!  * Add statistics for the total memory and free memory.
// * Refactor API to differentiate count and size of frames.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_set::BTreeSet;
use core::{alloc::Layout, array, cmp::min, ops::Range};

use ostd::mm::{page::allocator::PageAlloc, Paddr, PAGE_SIZE};

/// Buddy Frame allocator is a frame allocator based on buddy system, which
/// allocates memory in power-of-two sizes.
///
/// ## Introduction
///
/// The max order of the allocator is specified via the const generic parameter
/// `ORDER`. The frame allocator will only be able to allocate ranges of size
/// up to 2<sup>ORDER</sup>, out of a total range of size at most 2<sup>ORDER +
/// 1</sup> - 1.
pub struct BuddyFrameAllocator<const ORDER: usize = 32> {
    // buddy system with max order of ORDER
    free_list: [BTreeSet<usize>; ORDER],

    // statistics
    allocated: usize,
    total: usize,
}

pub(crate) fn prev_power_of_two(num: usize) -> usize {
    1 << (usize::BITS as usize - num.leading_zeros() as usize - 1)
}

impl<const ORDER: usize> BuddyFrameAllocator<ORDER> {
    /// Create an empty frame allocator
    pub fn new() -> Self {
        Self {
            free_list: array::from_fn(|_| BTreeSet::default()),
            allocated: 0,
            total: 0,
        }
    }

    /// Add a range of free pages, described by the **frame number**
    /// [start, end), for the allocator to manage.
    fn add_free_pages(&mut self, range: Range<usize>) {
        let start = range.start;
        let end = range.end;
        assert!(start <= end);

        let mut total = 0;
        let mut current_start = start;

        while current_start < end {
            let lowbit = if current_start > 0 {
                current_start & (!current_start + 1)
            } else {
                32
            };
            let size = min(
                min(lowbit, prev_power_of_two(end - current_start)),
                1 << (ORDER - 1),
            );
            total += size;

            self.free_list[size.trailing_zeros() as usize].insert(current_start);
            current_start += size;
        }

        self.total += total;
    }

    /// Allocate a range of frames from the allocator, returning the first frame of the allocated
    /// range.
    pub fn alloc(&mut self, count: usize) -> Option<usize> {
        self.alloc_power_of_two(count.next_power_of_two().trailing_zeros())
    }

    /// Allocate a range of frames of the given count's power of two from the
    /// allocator. The allocated range will have alignment equal to the power.
    fn alloc_power_of_two(&mut self, power: u32) -> Option<usize> {
        let class = power as usize;
        for i in class..self.free_list.len() {
            // Find the first non-empty size class
            if !self.free_list[i].is_empty() {
                // Split buffers
                for j in (class + 1..i + 1).rev() {
                    if let Some(block_ref) = self.free_list[j].iter().next() {
                        let block = *block_ref;
                        self.free_list[j - 1].insert(block + (1 << (j - 1)));
                        self.free_list[j - 1].insert(block);
                        self.free_list[j].remove(&block);
                    } else {
                        return None;
                    }
                }

                let result = self.free_list[class].iter().next();
                if let Some(result_ref) = result {
                    let result = *result_ref;
                    self.free_list[class].remove(&result);
                    self.allocated += 1 << class;
                    return Some(result);
                } else {
                    return None;
                }
            }
        }
        None
    }

    /// Deallocate a range of frames [frame, frame+count) from the frame
    /// allocator.
    ///
    /// The range should be exactly the same when it was allocated, as in heap
    /// allocator.
    ///
    /// # Safety
    ///
    /// Do not deallocate the same range twice.
    pub fn dealloc(&mut self, start_frame: usize, count: usize) {
        self.dealloc_power_of_two(start_frame, count.next_power_of_two().trailing_zeros())
    }

    /// Deallocate a range of frames described by count's power of two from the
    /// allocator.
    fn dealloc_power_of_two(&mut self, start_frame: usize, power: u32) {
        let class = power as usize;

        // Merge free buddy lists
        let mut current_ptr = start_frame;
        let mut current_class = class;
        while current_class < self.free_list.len() {
            let buddy = current_ptr ^ (1 << current_class);
            if self.free_list[current_class].remove(&buddy) {
                // Free buddy found
                current_ptr = min(current_ptr, buddy);
                current_class += 1;
            } else {
                self.free_list[current_class].insert(current_ptr);
                break;
            }
        }

        self.allocated -= 1 << class;
    }

    /// Given frames, described by a range of **frame number** [start, end),
    /// mark them as allocated. Make sure they can be correctly deallocated
    /// afterwards, while will not be allocated before deallocation.
    ///
    /// # Panics
    ///
    /// The function panics if no suitable block found for the given range.
    ///
    pub fn alloc_specific(&mut self, start: usize, end: usize) {
        let mut current_start = start;
        while current_start < end {
            /*
            Algorithm:

            1. Find one free block(begin_frame, class) already in the free
            list, which contains at least one frame described by
            current_start. If not, panic.

            2. Split the block corresponding to the buddy algorithm. Find
            the biggest sub-block which begins with current_start. The end of sublock should be smaller than end.
            */

            let mut size = 0;
            // # TODO
            //
            // We are working on a more efficient implementation, based on free meta
            // (i.e., current unused meta) and in the format of linked list. By
            // introducing the free meta, we can reduce the time complexity of
            // deleting blocks from O(log(n)) to O(1).
            for i in (0..self.free_list.len()).rev() {
                if self.free_list[i].is_empty() {
                    continue;
                }
                // Traverse the blocks in the btree list
                for block_iter in self.free_list[i].iter() {
                    let block = *block_iter;
                    // block means the start frame of the block
                    if block > current_start {
                        break;
                    }
                    if block <= current_start && block + (1 << i) > current_start {
                        if block == current_start && block + (1 << i) <= end {
                            self.free_list[i].remove(&block);
                            size = 1 << i;
                        } else if i > 0 {
                            self.free_list[i - 1].insert(block);
                            self.free_list[i - 1].insert(block + (1 << (i - 1)));
                            self.free_list[i].remove(&block);
                        }
                        break;
                    }
                }

                if size != 0 {
                    // Already found the suitable block
                    break;
                }
            }

            if size == 0 {
                panic!(
                    "No suitable block found for current_start: {:x}",
                    current_start
                );
            }

            current_start += size;
            // Update statistics
            self.allocated += size;
        }
    }
}

impl PageAlloc for BuddyFrameAllocator<32> {
    fn add_free_pages(&mut self, range: Range<usize>) {
        BuddyFrameAllocator::add_free_pages(self, range)
    }

    fn alloc(&mut self, layout: Layout) -> Option<Paddr> {
        assert!(layout.size() & (PAGE_SIZE - 1) == 0);
        BuddyFrameAllocator::alloc(self, layout.size() / PAGE_SIZE).map(|idx| idx * PAGE_SIZE)
    }

    fn dealloc(&mut self, addr: Paddr, nr_pages: usize) {
        BuddyFrameAllocator::dealloc(self, addr / PAGE_SIZE, nr_pages)
    }

    fn total_mem(&self) -> usize {
        self.total * PAGE_SIZE
    }

    fn free_mem(&self) -> usize {
        (self.total - self.allocated) * PAGE_SIZE
    }
}
