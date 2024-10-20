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
// * Use [`FreePage`] linked list to improve page management efficiency.
// * Add `alloc_specific` to allocate a specific range of frames.
// * Implement [`PageAlloc`] trait for `BuddyFrameAllocator`.
// * Add statistics for the total memory and free memory.
// * Refactor API to differentiate count and size of frames.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

use core::{alloc::Layout, array, cmp::min, ops::Range};

use ostd::{
    mm::{
        page::{allocator::PageAlloc, meta::FreePage},
        Paddr, PAGE_SIZE,
    },
    sync::SpinLock,
};

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
    // free_list keeps each class's entrance block's Frame Number
    // Use linked list provided `meta::FreeMeta` to implement corresponding
    // functions
    //
    // Notice: The value within the linked list represents the class of the
    // block, which is set to class + 1, to avoid misjudgment between class 0
    // and un-initialized block.
    free_list: [Paddr; ORDER],

    // statistics
    block_cnt: [u32; ORDER],
    allocated: usize,
    total: usize,
}

/// The locked version of the [`BuddyFrameAllocator`], a frame allocator based
/// on buddy system. Import [`SpinLock`] to get the inner mutable reference,
/// catering to the requirement of the `PageAlloc` trait. For more details of
/// buddy system, please refer to the [`BuddyFrameAllocator`] documentation.
pub struct LockedBuddyFrameAllocator<const ORDER: usize = 32> {
    // BuddyFrameAllocator with spinlock
    allocator: SpinLock<BuddyFrameAllocator<ORDER>>,
}

pub(crate) fn prev_power_of_two(num: usize) -> usize {
    1 << (usize::BITS as usize - num.leading_zeros() as usize - 1)
}

impl<const ORDER: usize> BuddyFrameAllocator<ORDER> {
    /// Create an empty frame allocator
    pub fn new() -> Self {
        Self {
            free_list: array::from_fn(|_| 0),
            block_cnt: array::from_fn(|_| 0),
            allocated: 0,
            total: 0,
        }
    }

    /// Insert a frame to the free list
    ///
    /// Panic
    ///
    /// - The function panics if the class is larger than the max order.
    pub(crate) fn insert_to_free_list(&mut self, class: usize, frame: usize) {
        assert!(class < ORDER);

        FreePage::try_lock(frame * PAGE_SIZE)
            .expect("Failed to lock FreePage while adding frame")
            .init(class + 1);
        if self.block_cnt[class] == 0 {
            self.free_list[class] = frame;
        } else {
            let mut list_head = FreePage::try_lock(self.free_list[class] * PAGE_SIZE)
                .expect("Failed to lock FreePage while adding frame");
            list_head.insert_before(frame * PAGE_SIZE);
        }
        self.block_cnt[class] += 1;
    }

    /// Remove a frame from the free list
    ///
    /// Panic
    ///
    /// The function panics if：
    /// - The class is larger than the max order.
    /// - The class is empty.
    pub(crate) fn remove_from_free_list(&mut self, class: usize, frame: usize) {
        assert!(class < ORDER);
        assert!(self.block_cnt[class] > 0);

        let mut freepage = FreePage::try_lock(frame * PAGE_SIZE)
            .expect("Failed to lock FreePage while removing frame");

        if frame == self.free_list[class] {
            if let Some(next) = freepage.next() {
                self.free_list[class] = next / PAGE_SIZE;
            } else if self.block_cnt[class] > 1 {
                let next = frame * PAGE_SIZE;
                let prev = freepage.prev().unwrap_or(frame * PAGE_SIZE);
                panic!("Free list is corrupted, class: {}, frame: {:x}, freelist cnt: {}, next: {:x}, prev: {:x}", class, frame, self.block_cnt[class], next, prev);
            }
        }

        freepage.remove();
        self.block_cnt[class] -= 1;
    }

    /// Check if the buddy of the given frame is free
    pub(crate) fn is_free_block(&self, frame: usize, class: usize) -> bool {
        if let Some(freepage) = FreePage::try_lock(frame * PAGE_SIZE) {
            freepage.value() == class + 1
        } else {
            false
        }
    }

    /// Add a range of free pages, described by the **frame number**
    /// [start, end), for the allocator to manage.
    pub(crate) fn add_free_pages(&mut self, range: Range<usize>) {
        let start = range.start;
        let end = range.end;
        assert!(start <= end);

        let mut total = 0;
        let mut current_start = start;
        // Segment length is the length of the current segment
        // Find the longest segment of free frames, improve the efficiency
        let mut segment_length = 0;

        while current_start + segment_length < end {
            if FreePage::try_lock((current_start + segment_length) * PAGE_SIZE).is_none() {
                if segment_length == 0 {
                    current_start += 1;
                    continue;
                }
            } else {
                segment_length += 1;
                if segment_length < (1 << (ORDER - 1)) && current_start + segment_length < end {
                    continue;
                }
            }

            let lowbit = if current_start > 0 {
                current_start & (!current_start + 1)
            } else {
                32
            };
            let size = min(
                min(lowbit, prev_power_of_two(segment_length)),
                1 << (ORDER - 1),
            );
            let cur_class = size.trailing_zeros() as usize;
            self.insert_to_free_list(cur_class, current_start);

            current_start += size;
            segment_length = 0;
            total += size;
        }

        self.total += total;
    }

    /// Allocate a range of frames from the allocator, returning the first frame of the allocated
    /// range.
    pub(crate) fn alloc(&mut self, count: usize) -> Option<usize> {
        self.alloc_power_of_two(count.next_power_of_two().trailing_zeros())
    }

    /// Allocate a range of frames of the given count's power of two from the
    /// allocator. The allocated range will have alignment equal to the power.
    fn alloc_power_of_two(&mut self, power: u32) -> Option<usize> {
        let class = power as usize;
        for i in class..self.free_list.len() {
            // Find the first non-empty size class
            if self.block_cnt[i] > 0 {
                // Split buffers
                for j in (class + 1..i + 1).rev() {
                    let block_frame = FreePage::try_lock(self.free_list[j] * PAGE_SIZE)
                        .expect("Failed to lock FreePage while allocating frame")
                        .prev()
                        .unwrap_or(self.free_list[j] * PAGE_SIZE)
                        / PAGE_SIZE;

                    // Remove original buffer from the class[j] list
                    self.remove_from_free_list(j, block_frame);

                    // Add two new buffers to the class[j-1] list
                    self.insert_to_free_list(j - 1, block_frame);
                    self.insert_to_free_list(j - 1, block_frame + (1 << (j - 1)));
                }

                // Allocate the buffer
                let result_frame = FreePage::try_lock(self.free_list[class] * PAGE_SIZE)
                    .expect("Failed to lock FreePage while allocating frame")
                    .prev()
                    .unwrap_or(self.free_list[class] * PAGE_SIZE)
                    / PAGE_SIZE;
                self.remove_from_free_list(class, result_frame);
                self.allocated += 1 << class;

                return Some(result_frame);
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
    pub(crate) fn dealloc(&mut self, start_frame: usize, count: usize) {
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
            if self.is_free_block(buddy, current_class) {
                // Free buddy found
                self.remove_from_free_list(current_class, buddy);
                current_ptr = min(current_ptr, buddy);
                current_class += 1;
            } else {
                self.insert_to_free_list(current_class, current_ptr);
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
    #[allow(unused)]
    pub(crate) fn alloc_specific(&mut self, start: usize, end: usize) {
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
                if self.block_cnt[i] == 0 {
                    continue;
                }
                // Traverse the blocks in the list
                let block = self.free_list[i];
                for _ in 0..self.block_cnt[i] {
                    // block means the start frame of the block
                    if block <= current_start && block + (1 << i) > current_start {
                        if block == current_start && block + (1 << i) <= end {
                            self.remove_from_free_list(i, block);
                            size = 1 << i;
                        } else if i > 0 {
                            self.insert_to_free_list(i - 1, block);
                            self.insert_to_free_list(i - 1, block + (1 << (i - 1)));
                            self.remove_from_free_list(i, block);
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

    pub(crate) fn total_mem(&self) -> usize {
        self.total * PAGE_SIZE
    }

    pub(crate) fn free_mem(&self) -> usize {
        (self.total - self.allocated) * PAGE_SIZE
    }
}

impl LockedBuddyFrameAllocator<32> {
    /// Create a new locked buddy frame allocator
    pub fn new() -> Self {
        Self {
            allocator: SpinLock::new(BuddyFrameAllocator::new()),
        }
    }
}

impl PageAlloc for LockedBuddyFrameAllocator<32> {
    fn add_free_pages(&self, range: Range<usize>) {
        self.allocator.disable_irq().lock().add_free_pages(range)
    }

    fn alloc(&self, layout: Layout) -> Option<Paddr> {
        assert!(layout.size() & (PAGE_SIZE - 1) == 0);
        self.allocator
            .disable_irq()
            .lock()
            .alloc(layout.size() / PAGE_SIZE)
            .map(|idx| idx * PAGE_SIZE)
    }

    fn dealloc(&self, addr: Paddr, nr_pages: usize) {
        self.allocator
            .disable_irq()
            .lock()
            .dealloc(addr / PAGE_SIZE, nr_pages)
    }

    fn total_mem(&self) -> usize {
        self.allocator.disable_irq().lock().total_mem()
    }

    fn free_mem(&self) -> usize {
        self.allocator.disable_irq().lock().free_mem()
    }
}
