// SPDX-License-Identifier: MPL-2.0

use ostd::{
    impl_frame_meta_for,
    mm::{frame::linked_list::Link, Paddr, UniqueFrame, PAGE_SIZE},
};

/// The order of a buddy chunk.
///
/// The size of a buddy chunk is `(1 << order) * PAGE_SIZE`.
pub(crate) type BuddyOrder = usize;

/// Returns the size of a buddy chunk of the given order.
pub(crate) const fn size_of_order(order: BuddyOrder) -> usize {
    (1 << order) * PAGE_SIZE
}

/// Returns an order that covers at least the given size.
///
/// The size must be larger than 0.
pub(crate) fn greater_order_of(size: usize) -> BuddyOrder {
    let size = size / PAGE_SIZE;
    size.next_power_of_two().trailing_zeros() as BuddyOrder
}

/// Returns a order that covers at most the given size.
///
/// The size must be larger than 0.
pub(crate) fn lesser_order_of(size: usize) -> BuddyOrder {
    let size = size / PAGE_SIZE;
    (usize::BITS - size.leading_zeros() - 1) as BuddyOrder
}

/// Splits a range into chunks.
///
/// A chunk must have a `1 << order` size and alignment, so a random page-
/// aligned range might not be a chunk.
///
/// This function returns an iterator that yields the set of chunks whose union
/// is the range, and the number of the chunks is the smallest.
///
/// # Panics
///
/// It panics if the address is not page-aligned.
pub(crate) fn split_to_chunks(
    addr: Paddr,
    size: usize,
) -> impl Iterator<Item = (Paddr, BuddyOrder)> {
    assert!(addr % PAGE_SIZE == 0);
    assert!(size % PAGE_SIZE == 0);

    struct SplitChunks {
        addr: Paddr,
        size: usize,
    }

    impl Iterator for SplitChunks {
        type Item = (Paddr, BuddyOrder);

        fn next(&mut self) -> Option<Self::Item> {
            if self.size == 0 {
                return None;
            }

            let order = max_order_from(self.addr).min(lesser_order_of(self.size));
            let chunk_size = size_of_order(order);
            let chunk_addr = self.addr;

            self.addr += chunk_size;
            self.size -= chunk_size;

            Some((chunk_addr, order))
        }
    }

    SplitChunks { addr, size }
}

/// Returns the maximum order starting from the address.
///
/// If the start address is not aligned to the order, the address/order pair
/// cannot form a buddy chunk.
///
/// # Panics
///
/// Panics if the address is not page-aligned in debug mode.
pub(crate) fn max_order_from(addr: Paddr) -> BuddyOrder {
    (addr.trailing_zeros() - PAGE_SIZE.trailing_zeros()) as BuddyOrder
}

/// Splits a large buddy chunk into two smaller buddies of order `split_order`.
///
/// Returns the addresses of each buddy.
///
/// # Panics
///
/// Panics if the address is not aligned to the `order`.
pub(crate) fn split_to_order(
    addr: Paddr,
    order: BuddyOrder,
    split_order: BuddyOrder,
) -> impl Iterator<Item = Paddr> {
    assert_eq!(addr % size_of_order(order), 0);

    let split_count = 1 << (order - split_order);
    let split_size = size_of_order(split_order);
    (0..split_count).map(move |i| addr + split_size * i)
}

/// The metadata of the head frame in a free buddy chunk.
#[derive(Debug)]
pub(crate) struct FreeHeadMeta {
    /// The order of the buddy chunk.
    order: BuddyOrder,
}

impl_frame_meta_for!(FreeHeadMeta);

impl FreeHeadMeta {
    /// Returns the order of the buddy chunk.
    pub(crate) fn order(&self) -> BuddyOrder {
        self.order
    }
}

/// A free buddy chunk.
#[derive(Debug)]
pub(crate) struct FreeChunk {
    head: UniqueFrame<Link<FreeHeadMeta>>,
}

impl FreeChunk {
    /// Gets a buddy chunk from the head frame.
    ///
    /// The caller must ensure that the head frame should be uniquely free.
    /// Otherwise it waits indefinitely.
    ///
    /// We need a unique ownership of this chunk. Other threads may be
    /// deallocating it's buddy and inspecting this chunk (see
    /// [`Self::buddy`]). So we may spuriously fail to acquire it. But
    /// they will soon release it so we can acquire it ultimately.
    pub(crate) fn from_free_head(head: UniqueFrame<Link<FreeHeadMeta>>) -> FreeChunk {
        FreeChunk { head }
    }

    /// Gets a buddy chunk from unused frames.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the range is not actually unused;
    ///  - the address is not aligned to the order.
    pub(crate) fn from_unused(addr: Paddr, order: BuddyOrder) -> FreeChunk {
        assert_eq!(addr % size_of_order(order), 0);

        let head = UniqueFrame::from_unused(addr, Link::new(FreeHeadMeta { order }))
            .expect("The head frame is not unused");

        #[cfg(debug_assertions)]
        {
            use ostd::mm::{
                frame::meta::{AnyFrameMeta, GetFrameError},
                Frame,
            };

            let end = addr + size_of_order(order);
            for paddr in (addr + PAGE_SIZE..end).step_by(PAGE_SIZE) {
                let Err(GetFrameError::Unused) = Frame::<dyn AnyFrameMeta>::from_in_use(paddr)
                else {
                    panic!("The range is not actually unused");
                };
            }
        }

        FreeChunk { head }
    }

    /// Turns the free chunk into a pointer to the head frame.
    pub(crate) fn into_unique_head(self) -> UniqueFrame<Link<FreeHeadMeta>> {
        self.head
    }

    /// Returns the order of the buddy chunk.
    pub(crate) fn order(&self) -> BuddyOrder {
        self.head.meta().order()
    }

    /// Returns the address of the buddy chunk.
    pub(crate) fn addr(&self) -> Paddr {
        self.head.start_paddr()
    }

    /// Gets the address of the buddy of this chunk.
    pub(crate) fn buddy(&self) -> Paddr {
        let addr = self.addr();
        let order = self.order();
        addr ^ size_of_order(order)
    }

    /// Splits the buddy chunk into two smaller buddies.
    ///
    /// # Panics
    ///
    /// Panics if the buddy chunk is not uniquely free.
    pub(crate) fn split_free(self) -> (FreeChunk, FreeChunk) {
        let order = self.order();
        let addr = self.addr();
        let new_order = order - 1;
        let left_child_addr = addr;
        let right_child_addr = addr ^ size_of_order(new_order);

        let mut unique_head = self.into_unique_head();
        debug_assert_eq!(unique_head.start_paddr(), left_child_addr);
        unique_head.meta_mut().order = new_order;

        let left_child = FreeChunk { head: unique_head };
        let right_child = FreeChunk {
            head: UniqueFrame::from_unused(
                right_child_addr,
                Link::new(FreeHeadMeta { order: new_order }),
            )
            .expect("Tail frames are not unused"),
        };
        (left_child, right_child)
    }

    /// Merges the buddy chunk with the sibling buddy.
    ///
    /// # Panics
    ///
    /// Panics if either the buddy chunks are not free or not buddies.
    pub(crate) fn merge_free(mut self, mut buddy: FreeChunk) -> FreeChunk {
        if self.addr() > buddy.addr() {
            core::mem::swap(&mut self, &mut buddy);
        }

        let order = self.order();
        let addr = self.addr();
        let buddy_order = buddy.order();
        let buddy_addr = buddy.addr();

        buddy.into_unique_head().reset_as_unused(); // This will "drop" the frame without up-calling us.

        assert_eq!(order, buddy_order);
        assert_eq!(addr ^ size_of_order(order), buddy_addr);
        let new_order = order + 1;
        let mut unique_head = self.into_unique_head();
        unique_head.meta_mut().order = new_order;
        FreeChunk { head: unique_head }
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::test::MockMemoryRegion;
    use ostd::prelude::ktest;

    #[ktest]
    fn test_greater_order_of() {
        #[track_caller]
        fn assert_greater_order_of(nframes: usize, expected: BuddyOrder) {
            assert_eq!(greater_order_of(nframes * PAGE_SIZE), expected);
        }

        assert_greater_order_of(1, 0);
        assert_greater_order_of(2, 1);
        assert_greater_order_of(3, 2);
        assert_greater_order_of(4, 2);
        assert_greater_order_of(5, 3);
        assert_greater_order_of(6, 3);
        assert_greater_order_of(7, 3);
        assert_greater_order_of(8, 3);
        assert_greater_order_of(9, 4);
    }

    #[ktest]
    fn test_lesser_order_of() {
        #[track_caller]
        fn assert_lesser_order_of(nframes: usize, expected: BuddyOrder) {
            assert_eq!(lesser_order_of(nframes * PAGE_SIZE), expected);
        }

        assert_lesser_order_of(1, 0);
        assert_lesser_order_of(2, 1);
        assert_lesser_order_of(3, 1);
        assert_lesser_order_of(4, 2);
        assert_lesser_order_of(5, 2);
        assert_lesser_order_of(6, 2);
        assert_lesser_order_of(7, 2);
        assert_lesser_order_of(8, 3);
        assert_lesser_order_of(9, 3);
    }

    #[ktest]
    fn test_max_order_from() {
        #[track_caller]
        fn assert_max_order_from(frame_num: usize, expected: BuddyOrder) {
            assert_eq!(max_order_from(frame_num * PAGE_SIZE), expected);
        }

        assert_max_order_from(0, (usize::BITS - PAGE_SIZE.trailing_zeros()) as BuddyOrder);
        assert_max_order_from(1, 0);
        assert_max_order_from(2, 1);
        assert_max_order_from(3, 0);
        assert_max_order_from(4, 2);
        assert_max_order_from(5, 0);
        assert_max_order_from(6, 1);
        assert_max_order_from(7, 0);
        assert_max_order_from(8, 3);
        assert_max_order_from(9, 0);
        assert_max_order_from(10, 1);
        assert_max_order_from(11, 0);
        assert_max_order_from(12, 2);
    }

    #[ktest]
    fn test_split_to_chunks() {
        use alloc::{vec, vec::Vec};

        #[track_caller]
        fn assert_split_to_chunk(
            addr_frame_num: usize,
            size_num_frames: usize,
            expected: Vec<(Paddr, BuddyOrder)>,
        ) {
            let addr = addr_frame_num * PAGE_SIZE;
            let size = size_num_frames * PAGE_SIZE;
            let chunks: Vec<_> = split_to_chunks(addr, size).collect();

            let expected = expected
                .iter()
                .map(|(addr, order)| (addr * PAGE_SIZE, *order))
                .collect::<Vec<_>>();

            assert_eq!(chunks, expected);
        }

        assert_split_to_chunk(0, 0, vec![]);
        assert_split_to_chunk(0, 1, vec![(0, 0)]);
        assert_split_to_chunk(0, 2, vec![(0, 1)]);
        assert_split_to_chunk(6, 32, vec![(6, 1), (8, 3), (16, 4), (32, 2), (36, 1)]);
        assert_split_to_chunk(7, 5, vec![(7, 0), (8, 2)]);
        assert_split_to_chunk(12, 16, vec![(12, 2), (16, 3), (24, 2)]);
        assert_split_to_chunk(1024, 1024, vec![(1024, 10)]);
    }

    #[ktest]
    fn test_split_to_order() {
        use alloc::{vec, vec::Vec};

        #[track_caller]
        fn assert_split_to_order(
            addr_frame_num: usize,
            order: BuddyOrder,
            split_order: BuddyOrder,
            expected: Vec<Paddr>,
        ) {
            let addr = addr_frame_num * PAGE_SIZE;
            let chunks: Vec<_> = split_to_order(addr, order, split_order).collect();

            let expected = expected
                .iter()
                .map(|addr| addr * PAGE_SIZE)
                .collect::<Vec<_>>();

            assert_eq!(chunks, expected);
        }

        assert_split_to_order(0, 3, 3, vec![0]);
        assert_split_to_order(0, 3, 2, vec![0, 4]);
        assert_split_to_order(0, 3, 1, vec![0, 2, 4, 6]);
        assert_split_to_order(0, 3, 0, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[ktest]
    fn test_free_chunk_ops() {
        let order = 3;
        let size = size_of_order(order);
        let region = MockMemoryRegion::alloc(size);
        let addr1 = region.start_paddr();
        let addr2 = addr1 + size_of_order(order - 2);
        let addr3 = addr1 + size_of_order(order - 2) * 2;

        let chunk = FreeChunk::from_unused(addr1, order);
        assert_eq!(chunk.order(), order);
        assert_eq!(chunk.addr(), addr1);
        assert_eq!(chunk.buddy(), addr1 ^ size);

        let (left, right) = chunk.split_free();

        assert_eq!(left.order(), order - 1);
        assert_eq!(left.addr(), addr1);
        assert_eq!(left.buddy(), addr3);

        assert_eq!(right.order(), order - 1);
        assert_eq!(right.addr(), addr3);
        assert_eq!(right.buddy(), addr1);

        let (r1, r2) = left.split_free();

        assert_eq!(r1.order(), order - 2);
        assert_eq!(r1.addr(), addr1);
        assert_eq!(r1.buddy(), addr2);

        assert_eq!(r2.order(), order - 2);
        assert_eq!(r2.addr(), addr2);
        assert_eq!(r2.buddy(), addr1);

        let left = r1.merge_free(r2);
        let chunk = left.merge_free(right);
        assert_eq!(chunk.order(), order);
        assert_eq!(chunk.addr(), addr1);

        chunk.into_unique_head().reset_as_unused();
    }
}
