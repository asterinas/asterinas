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
//
// * Establish the original allocator as the primary central free list and
//   implement a per-CPU cache to minimize contention and enhance efficiency in
//   a concurrent environment.
// * Use [`FreePage`] linked list to improve page management efficiency with
//   safe code.
// * Add `alloc_specific` to allocate a specific range of frames.
// * Implement [`PageAlloc`] trait for `BuddyFrameAllocator`.
// * Add statistics for the total memory and free memory.
// * Refactor API to differentiate count and size of frames.
//
// These changes are released under the following license:
//
// SPDX-License-Identifier: MPL-2.0

use core::{
    alloc::Layout,
    array,
    cell::RefCell,
    cmp::min,
    ops::Range,
    sync::atomic::{self, Ordering},
};

use log::warn;
use ostd::{
    cpu_local,
    mm::{
        page::{allocator::PageAlloc, meta::FreePage},
        Paddr, PAGE_SIZE,
    },
    sync::SpinLock,
    trap::{self},
};

/// Config of the buddy system
///
/// 1. Order of the buddy system, i.e., the number of classes of free blocks
///
/// 2. The threshold of the buddy system's per-cpu cache.
///    - Since large blocks are rarely allocated, caching such blocks will waste
///      CPU local cache.
///    - If current task is applying for more than the threshold, the page
///      allocator will bypass the CPU local cache and directly allocate from
///      the central free list.
///   -  **Notice**: The threshold should be less than the max order of the
///      buddy system.
///
/// 3. The **page number** of the CPU local cache.
///    - The max and min size of the CPU local cache is used as the threshold of
///      the buddy system's per-cpu cache.
const BUDDY_ORDER: usize = 32;
const THRESHOLD: usize = 14;
const CPU_LOCAL_PAGE_COUNT: usize = 1 << THRESHOLD; // 64MB, 16K 4KB pages
const MAX_LOCAL_PAGE_COUNT: usize = CPU_LOCAL_PAGE_COUNT * 2;

/// Buddy Free List is set of lists that store free blocks of different sizes,
/// which ulitizes the buddy system to manage the free blocks. The list itself
/// also work as a frame allocator, which can allocate and deallocate in
/// power-of-two sizes.
///
/// ## Allocation Details
///
/// The max order of the list is specified via the const generic parameter
/// `ORDER`. The list will only be able to allocate ranges of size up to
/// 2<sup>ORDER</sup>, out of a total range of size at most 2<sup>ORDER +
/// 1</sup> - 1.
#[derive(Debug)]
pub struct BuddyFreeList<const ORDER: usize = BUDDY_ORDER> {
    // [`free_list`] keeps each class's entrance block's Frame Number
    // Use linked list provided `meta::FreeMeta` to implement corresponding
    // functions
    //
    // Notice: The value within the linked list represents the class of the
    // block, which is set to class + 1, to avoid misjudgment between class 0
    // and un-initialized block.
    free_lists: [Paddr; ORDER],

    // statistics
    block_cnt: [u32; ORDER],
    total: usize,
    free: usize,
}

/// The [`BuddyFrameAllocator`] is a concurrent allocator based on buddy system.
/// The allocator is made up of a central free list [`BuddyFreeList`] and a
/// per-CPU cache described by [`LOCAL_FREE_LISTS`] and
/// [`LOCAL_FREE_LISTS_CNT`]. For more details of buddy system, please refer to
/// the [`BuddyFreeList`] documentation.
#[derive(Debug)]
pub struct BuddyFrameAllocator<const ORDER: usize = BUDDY_ORDER> {
    // Central freelist with spinlock
    central_free_lists: SpinLock<BuddyFreeList<ORDER>>,

    // statistics
    total: atomic::AtomicUsize,
    free: atomic::AtomicUsize,
}

cpu_local! {
    /// Local freelist for each CPU, designed rapid page allocation
    /// for multi-core system.
    pub(crate) static LOCAL_FREE_LISTS:RefCell<[Paddr; THRESHOLD]> = RefCell::new([0; THRESHOLD]);
    /// Corresponding free block count for each class.
    pub(crate) static LOCAL_FREE_LISTS_CNT:RefCell<[u32; THRESHOLD]> = RefCell::new([0; THRESHOLD]);

}

pub(crate) fn prev_power_of_two(num: usize) -> usize {
    1 << (usize::BITS as usize - num.leading_zeros() as usize - 1)
}

impl<const ORDER: usize> BuddyFreeList<ORDER> {
    /// Create an empty buddy free list
    pub fn new() -> Self {
        Self {
            free_lists: array::from_fn(|_| 0),
            block_cnt: array::from_fn(|_| 0),
            free: 0,
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
            self.free_lists[class] = frame;
        } else {
            let mut list_head = FreePage::try_lock(self.free_lists[class] * PAGE_SIZE)
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

        if frame == self.free_lists[class] {
            if let Some(next) = freepage.next() {
                self.free_lists[class] = next / PAGE_SIZE;
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
    pub(crate) fn add_free_pages(&mut self, range: Range<usize>) -> usize {
        let start = range.start;
        let end = range.end;
        assert!(start <= end);

        let mut added_pages = 0;
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
            added_pages += size;
        }

        self.total += range.end - range.start;
        self.free += added_pages;

        added_pages
    }

    /// Allocate a range of frames from the allocator, returning the first
    /// frame of the allocated range.
    pub(crate) fn alloc(&mut self, count: usize) -> Option<usize> {
        self.alloc_power_of_two(count.next_power_of_two().trailing_zeros())
    }

    /// Allocate a range of frames of the given count's power of two from the
    /// allocator. The allocated range will have alignment equal to the power.
    fn alloc_power_of_two(&mut self, power: u32) -> Option<usize> {
        let class = power as usize;
        for i in class..self.free_lists.len() {
            // Find the first non-empty size class
            if self.block_cnt[i] > 0 {
                // Split buffers
                for j in (class + 1..i + 1).rev() {
                    let block_frame = FreePage::try_lock(self.free_lists[j] * PAGE_SIZE)
                        .expect("Failed to lock FreePage while allocating frame")
                        .prev()
                        .unwrap_or(self.free_lists[j] * PAGE_SIZE)
                        / PAGE_SIZE;

                    // Remove original buffer from the class[j] list
                    self.remove_from_free_list(j, block_frame);

                    // Add two new buffers to the class[j-1] list
                    self.insert_to_free_list(j - 1, block_frame);
                    self.insert_to_free_list(j - 1, block_frame + (1 << (j - 1)));
                }

                // Allocate the buffer
                let result_frame = FreePage::try_lock(self.free_lists[class] * PAGE_SIZE)
                    .expect("Failed to lock FreePage while allocating frame in free list")
                    .prev()
                    .unwrap_or(self.free_lists[class] * PAGE_SIZE)
                    / PAGE_SIZE;
                self.remove_from_free_list(class, result_frame);
                self.free -= 1 << class;

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
        while current_class < self.free_lists.len() {
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

        self.free += 1 << class;
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
            for i in (0..self.free_lists.len()).rev() {
                if self.block_cnt[i] == 0 {
                    continue;
                }
                // Traverse the blocks in the list
                let block = self.free_lists[i];
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
            self.free -= size;
        }
    }

    /// Returns the total number of bytes managed by the freelist.
    #[allow(unused)]
    pub(crate) fn total_mem(&self) -> usize {
        self.total * PAGE_SIZE
    }

    /// Returns the total number of bytes available for freelist's allocation.
    #[allow(unused)]
    pub(crate) fn free_mem(&self) -> usize {
        self.free * PAGE_SIZE
    }
}

impl<const ORDER: usize> BuddyFrameAllocator<ORDER> {
    /// Create a new buddy frame allocator
    pub fn new() -> Self {
        Self {
            central_free_lists: SpinLock::new(BuddyFreeList::new()),
            total: atomic::AtomicUsize::new(0),
            free: atomic::AtomicUsize::new(0),
        }
    }

    fn insert_to_local_cache(
        &self,
        class: usize,
        frame: usize,
        local_free_list: &mut [usize; THRESHOLD],
        local_free_list_cnt: &mut [u32; THRESHOLD],
    ) -> bool {
        assert!(class < ORDER);
        assert!(class < THRESHOLD);

        let cnt = local_free_list_cnt[class];
        let max_quota = MAX_LOCAL_PAGE_COUNT / (1 << class);
        if cnt < max_quota as u32 {
            if local_free_list_cnt[class] == 0 {
                local_free_list[class] = frame;
                FreePage::try_lock(frame * PAGE_SIZE)
                    .expect("Failed to lock FreePage while inserting to local cache")
                    .init(class + 1);
            } else {
                let mut freepage = FreePage::try_lock(local_free_list[class] * PAGE_SIZE)
                    .expect("Failed to lock FreePage while inserting to local cache");
                freepage.insert_before(frame * PAGE_SIZE);
            }
            local_free_list_cnt[class] += 1;

            true
        } else {
            warn!("Local cache is full, frame: {:x}, class: {}", frame, class);
            false
        }
    }

    fn remove_from_local_cache(
        &self,
        class: usize,
        frame: usize,
        local_free_list: &mut [usize; THRESHOLD],
        local_free_list_cnt: &mut [u32; THRESHOLD],
    ) {
        assert!(class < ORDER);
        assert!(class < THRESHOLD);
        assert!(local_free_list_cnt[class] > 0);

        let mut freepage = FreePage::try_lock(frame * PAGE_SIZE)
            .expect("Failed to lock FreePage while removing from local cache");

        if frame == local_free_list[class] {
            if let Some(next) = freepage.next() {
                local_free_list[class] = next / PAGE_SIZE;
            } else if local_free_list_cnt[class] > 1 {
                let next = frame * PAGE_SIZE;
                let prev = freepage.prev().unwrap_or(frame * PAGE_SIZE);
                panic!("Local free list is corrupted, class: {}, frame: {:x}, freelist cnt: {}, next: {:x}, prev: {:x}", class, frame, local_free_list_cnt[class], next, prev);
            }
        }

        freepage.remove();
        local_free_list_cnt[class] -= 1;
    }

    /// Decrease the local cache of the current CPU till the count of the
    /// specific class is less than the threshold.
    fn decrease_local_cache(
        &self,
        class: usize,
        count_threshold: u32,
        local_free_list: &mut [usize; THRESHOLD],
        local_free_list_cnt: &mut [u32; THRESHOLD],
    ) {
        let mut central_free_list = self.central_free_lists.disable_irq().lock();
        let front_frame = local_free_list[class];
        let front_freepage = FreePage::try_lock(front_frame * PAGE_SIZE)
            .expect("Fail to lock freepage while decreasing local cache");

        while local_free_list_cnt[class] > count_threshold {
            let target_frame = front_freepage.prev().unwrap_or(front_frame * PAGE_SIZE) / PAGE_SIZE;
            central_free_list.dealloc(target_frame, 1 << class);
            self.remove_from_local_cache(class, target_frame, local_free_list, local_free_list_cnt);
        }
    }

    /// Rescue the current CPU's local cache while the cache is running out.
    ///
    /// Allocate arrange of pages sized [`CPU_LOCAL_CACHE_SIZE`] from the
    /// central free list and put them into the local cache. The allocated
    /// pages will be split and saved into the local cache's freelist of
    /// specific class.
    ///
    /// # Warning
    ///
    /// The function will be panic if the input class is larger than the max
    /// order.
    pub fn rescue_local_cache(
        &self,
        class: usize,
        local_free_list: &mut [usize; THRESHOLD],
        local_free_list_cnt: &mut [u32; THRESHOLD],
    ) {
        assert!(class < ORDER);
        let new_frame_head: usize = if let Some(frame_idx) = self
            .central_free_lists
            .disable_irq()
            .lock()
            .alloc(CPU_LOCAL_PAGE_COUNT)
        {
            frame_idx
        } else {
            // The central free list is running out, recover the central
            // free list
            self.recover_central_freelist();
            // Retry to allocate from the central free list
            self.central_free_lists
                .disable_irq()
                .lock()
                .alloc(CPU_LOCAL_PAGE_COUNT)
                .expect("The system is out of memory.")
        };

        // Split the new pages and put them into the local cache
        for i in (0..CPU_LOCAL_PAGE_COUNT).step_by(1 << class) {
            let frame = new_frame_head + i;
            if !self.insert_to_local_cache(class, frame, local_free_list, local_free_list_cnt) {
                self.central_free_lists
                    .disable_irq()
                    .lock()
                    .dealloc(frame, CPU_LOCAL_PAGE_COUNT - i);
                break;
            }
        }
    }

    /// Recover the central free list by merging the local cache of all CPUs.
    ///
    /// Traverse all the local caches and merge the free blocks into the central
    /// free list. The function will be called when the central free list is
    /// running out.
    ///
    /// Since the size of the local free list is maintained when allocating and
    /// deallocating, there is tiny chance that the central free list is
    /// running out because of the overuse of the local cache. Therefore, it is
    /// an emergency when the central free list is running out and it is of
    /// great necessity to recycle all the local cache.
    pub fn recover_central_freelist(&self) {
        warn!(
            "Recover central freelist, local caches at current CPU will be merged into the central freelist."
        );
        // TODO: Since the tuple is not `Sync`, we temporarily recycle the
        // current CPU's local cache.
        let irq_guard = trap::disable_local();
        let list_guard = LOCAL_FREE_LISTS.get_with(&irq_guard);
        let cnt_guard = LOCAL_FREE_LISTS_CNT.get_with(&irq_guard);
        let mut local_free_list = list_guard
            .try_borrow_mut()
            .expect("Failed to borrow local free list");
        let mut local_free_list_cnt = cnt_guard
            .try_borrow_mut()
            .expect("Failed to borrow local free list count");

        for i in 0..ORDER {
            if local_free_list_cnt[i] > 0 {
                self.decrease_local_cache(i, 0, &mut local_free_list, &mut local_free_list_cnt);
            }
        }
    }

    /// Allocate a range of frames from the allocator, returning the first
    /// frame of the allocated range.
    pub fn alloc(&self, count: usize) -> Option<usize> {
        let class = count.next_power_of_two().trailing_zeros() as usize;

        if class >= THRESHOLD {
            // Bypass the local cache
            let frame = self.central_free_lists.disable_irq().lock().alloc(count)?;
            self.free.fetch_sub(1 << class, Ordering::Relaxed);
            return Some(frame);
        }

        // Try to allocate from the local cache
        let irq_guard = trap::disable_local();
        let cnt_guard = LOCAL_FREE_LISTS_CNT.get_with(&irq_guard);
        let mut local_free_list_cnt = cnt_guard
            .try_borrow_mut()
            .expect("Failed to borrow local free list count");

        let list_guard = LOCAL_FREE_LISTS.get_with(&irq_guard);
        let mut local_free_list = list_guard
            .try_borrow_mut()
            .expect("Failed to borrow local free list");

        if local_free_list_cnt[class] == 0 {
            // Local cache is empty, rescue the local cache
            self.rescue_local_cache(class, &mut local_free_list, &mut local_free_list_cnt);
        }

        let result_frame = FreePage::try_lock(local_free_list[class] * PAGE_SIZE)
            .expect("Failed to lock FreePage while allocating frame in local cache")
            .prev()
            .unwrap_or(local_free_list[class] * PAGE_SIZE)
            / PAGE_SIZE;

        self.remove_from_local_cache(
            class,
            result_frame,
            &mut local_free_list,
            &mut local_free_list_cnt,
        );

        self.free.fetch_sub(1 << class, Ordering::Relaxed);

        Some(result_frame)
    }

    /// Deallocate a range of frames [frame, frame+count) from the frame
    /// allocator.
    pub fn dealloc(&self, start_frame: usize, count: usize) {
        let class = count.next_power_of_two().trailing_zeros() as usize;

        if class >= THRESHOLD {
            self.central_free_lists
                .disable_irq()
                .lock()
                .dealloc(start_frame, count);
            self.free.fetch_add(count, Ordering::Relaxed);
            return;
        }

        let irq_guard = trap::disable_local();
        let cnt_guard = LOCAL_FREE_LISTS_CNT.get_with(&irq_guard);
        let mut local_free_list_cnt = cnt_guard
            .try_borrow_mut()
            .expect("Failed to borrow local free list count");

        let list_guard = LOCAL_FREE_LISTS.get_with(&irq_guard);
        let mut local_free_list = list_guard
            .try_borrow_mut()
            .expect("Failed to borrow local free list");

        if !self.insert_to_local_cache(
            class,
            start_frame,
            &mut local_free_list,
            &mut local_free_list_cnt,
        ) {
            self.central_free_lists
                .disable_irq()
                .lock()
                .dealloc(start_frame, count);
            // Since current local cache is full, we need to decrease the local
            // cache to its normal size
            let quota = CPU_LOCAL_PAGE_COUNT / (1 << class);
            self.decrease_local_cache(
                class,
                quota as u32,
                &mut local_free_list,
                &mut local_free_list_cnt,
            );
        }

        self.free.fetch_add(1 << class, Ordering::Relaxed);
    }
}

impl PageAlloc for BuddyFrameAllocator<BUDDY_ORDER> {
    fn add_free_pages(&self, range: Range<usize>) {
        // By default, the function only adds the range to the central free
        // list.
        let total_pages = range.end - range.start;
        let added_pages = self
            .central_free_lists
            .disable_irq()
            .lock()
            .add_free_pages(range);

        self.total.fetch_add(total_pages, Ordering::Relaxed);
        self.free.fetch_add(added_pages, Ordering::Relaxed);
    }

    fn alloc(&self, layout: Layout) -> Option<Paddr> {
        assert!(layout.size() & (PAGE_SIZE - 1) == 0);
        BuddyFrameAllocator::alloc(self, layout.size() / PAGE_SIZE).map(|idx| idx * PAGE_SIZE)
    }

    fn dealloc(&self, addr: Paddr, nr_pages: usize) {
        assert!(addr & (PAGE_SIZE - 1) == 0);
        BuddyFrameAllocator::dealloc(self, addr / PAGE_SIZE, nr_pages)
    }

    fn total_mem(&self) -> usize {
        self.total.load(Ordering::Relaxed) * PAGE_SIZE
    }

    fn free_mem(&self) -> usize {
        self.free.load(Ordering::Relaxed) * PAGE_SIZE
    }
}
