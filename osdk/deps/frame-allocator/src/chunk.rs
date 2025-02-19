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
pub(crate) fn greater_order_of(size: usize) -> BuddyOrder {
    let size = size / PAGE_SIZE;
    size.next_power_of_two().trailing_zeros() as BuddyOrder
}

/// Returns a order that covers at most the given size.
pub(crate) fn lesser_order_of(size: usize) -> BuddyOrder {
    let size = size / PAGE_SIZE;
    (usize::BITS - size.leading_zeros() - 1) as BuddyOrder
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
        assert!(addr % size_of_order(order) == 0);

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
