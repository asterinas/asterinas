// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{frame::linked_list::LinkedList, Paddr};

use crate::chunk::{size_of_order, BuddyOrder, FreeChunk, FreeHeadMeta};

/// A set of free buddy chunks.
pub(crate) struct BuddySet<const MAX_ORDER: BuddyOrder> {
    /// The sum of the sizes of all free chunks.
    total_size: usize,
    /// The lists of free buddy chunks for each orders.
    lists: [LinkedList<FreeHeadMeta>; MAX_ORDER],
}

impl<const MAX_ORDER: BuddyOrder> BuddySet<MAX_ORDER> {
    /// Creates a new empty set of free lists.
    pub(crate) const fn new_empty() -> Self {
        Self {
            total_size: 0,
            lists: [const { LinkedList::new() }; MAX_ORDER],
        }
    }

    /// Gets the total size of free chunks.
    pub(crate) fn total_size(&self) -> usize {
        self.total_size
    }

    /// Inserts a free chunk into the set.
    pub(crate) fn insert_chunk(&mut self, addr: Paddr, order: BuddyOrder) {
        debug_assert!(order < MAX_ORDER);

        let inserted_size = size_of_order(order);
        let mut chunk = FreeChunk::from_unused(addr, order);

        let order = chunk.order();
        // Coalesce the chunk with its buddy whenever possible.
        for (i, list) in self.lists.iter_mut().enumerate().skip(order) {
            if i + 1 >= MAX_ORDER {
                // The chunk is already the largest one.
                break;
            }
            let buddy_addr = chunk.buddy();
            let Some(mut cursor) = list.cursor_mut_at(buddy_addr) else {
                // The buddy is not in this free list, so we can't coalesce.
                break;
            };
            let taken = cursor.take_current().unwrap();
            debug_assert_eq!(buddy_addr, taken.start_paddr());
            chunk = chunk.merge_free(FreeChunk::from_free_head(taken));
        }
        // Insert the coalesced chunk into the free lists.
        let order = chunk.order();
        self.lists[order].push_front(chunk.into_unique_head());

        self.total_size += inserted_size;
    }

    /// Allocates a chunk from the set.
    ///
    /// The function will choose and remove a buddy chunk of the given order
    /// from the set. The address of the chunk will be returned.
    pub(crate) fn alloc_chunk(&mut self, order: BuddyOrder) -> Option<Paddr> {
        // Find the first non-empty size class larger than the requested order.
        let mut non_empty = None;
        for (i, list) in self.lists.iter_mut().enumerate().skip(order) {
            if !list.is_empty() {
                non_empty = Some(i);
                break;
            }
        }
        let non_empty = non_empty?;
        let mut chunk = {
            let head = self.lists[non_empty].pop_front().unwrap();
            debug_assert_eq!(head.meta().order(), non_empty as BuddyOrder);

            Some(FreeChunk::from_free_head(head))
        };

        // Split the chunk.
        for i in (order + 1..=non_empty).rev() {
            let (left_sub, right_sub) = chunk.take().unwrap().split_free();
            // Push the right sub-chunk back to the free lists.
            let right_sub = right_sub.into_unique_head();
            debug_assert_eq!(right_sub.meta().order(), (i - 1) as BuddyOrder);
            self.lists[i - 1].push_front(right_sub);
            // Pass the left sub-chunk to the next iteration.
            chunk = Some(left_sub);
        }

        let allocated_size = size_of_order(order);

        self.total_size -= allocated_size;

        // The remaining chunk is the one we want.
        let head_frame = chunk.take().unwrap().into_unique_head();
        let paddr = head_frame.start_paddr();
        head_frame.reset_as_unused(); // It will "drop" the frame without up-calling us.
        Some(paddr)
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::test::MockMemoryRegion;
    use ostd::prelude::ktest;

    #[ktest]
    fn test_buddy_set_insert_alloc() {
        let region_order = 4;
        let region_size = size_of_order(region_order);
        let region = MockMemoryRegion::alloc(region_size);
        let region_start = region.start_paddr();

        let mut set = BuddySet::<5>::new_empty();
        set.insert_chunk(region_start, region_order);
        assert!(set.total_size() == region_size);

        // Allocating chunks of orders of 0, 0, 1, 2, 3 should be okay.
        let chunk1 = set.alloc_chunk(0).unwrap();
        assert!(set.total_size() == region_size - size_of_order(0));
        let chunk2 = set.alloc_chunk(0).unwrap();
        assert!(set.total_size() == region_size - size_of_order(1));
        let chunk3 = set.alloc_chunk(1).unwrap();
        assert!(set.total_size() == region_size - size_of_order(2));
        let chunk4 = set.alloc_chunk(2).unwrap();
        assert!(set.total_size() == region_size - size_of_order(3));
        let chunk5 = set.alloc_chunk(3).unwrap();
        assert!(set.total_size() == 0);

        // Putting them back should enable us to allocate the original region.
        set.insert_chunk(chunk3, 1);
        assert!(set.total_size() == size_of_order(1));
        set.insert_chunk(chunk1, 0);
        assert!(set.total_size() == size_of_order(0) + size_of_order(1));
        set.insert_chunk(chunk5, 3);
        assert!(set.total_size() == size_of_order(0) + size_of_order(1) + size_of_order(3));
        set.insert_chunk(chunk2, 0);
        assert!(set.total_size() == size_of_order(2) + size_of_order(3));
        set.insert_chunk(chunk4, 2);
        assert!(set.total_size() == size_of_order(4));

        let chunk = set.alloc_chunk(region_order).unwrap();
        assert!(chunk == region_start);
        assert!(set.total_size() == 0);
    }
}
