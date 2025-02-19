// SPDX-License-Identifier: MPL-2.0

use ostd::mm::{frame::linked_list::LinkedList, Paddr};

use crate::chunk::{size_of_order, BuddyOrder, FreeChunk, FreeHeadMeta};

/// A set of free buddy chunks.
pub(crate) struct BuddySet<const MAX_ORDER: BuddyOrder>
where
    [(); MAX_ORDER as usize]:,
{
    /// The sum of the sizes of all free chunks.
    total_size: usize,
    /// The lists of free buddy chunks for each orders.
    lists: [LinkedList<FreeHeadMeta>; MAX_ORDER as usize],
}

impl<const MAX_ORDER: BuddyOrder> BuddySet<MAX_ORDER>
where
    [(); MAX_ORDER as usize]:,
{
    /// Create a new empty set of free lists.
    pub(crate) const fn new_empty() -> Self {
        Self {
            total_size: 0,
            lists: [const { LinkedList::new() }; MAX_ORDER as usize],
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
        for (i, list) in self.lists.iter_mut().enumerate().skip(order as usize) {
            if i + 1 >= MAX_ORDER as usize {
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
        self.lists[order as usize].push_front(chunk.into_unique_head());

        self.total_size += inserted_size;
    }

    /// Allocate a chunk from the set.
    ///
    /// The function will choose and remove a buddy chunk of the given order
    /// from the set. The address of the chunk will be returned.
    pub(crate) fn alloc_chunk(&mut self, order: BuddyOrder) -> Option<Paddr> {
        // Find the first non-empty size class larger than the requested order.
        let mut non_empty = None;
        for (i, list) in self.lists.iter_mut().enumerate().skip(order as usize) {
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
        for i in (order as usize + 1..=non_empty).rev() {
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
