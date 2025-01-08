// SPDX-License-Identifier: MPL-2.0

use ostd::{
    impl_frame_meta_for,
    mm::{
        frame::{
            linked_list::Link,
            meta::{AnyFrameMeta, GetFrameError},
        },
        Frame, Paddr, UniqueFrame, PAGE_SIZE,
    },
};

/// The order of a buddy chunk.
///
/// The size of a buddy chunk is `(1 << order) * PAGE_SIZE`.
pub(crate) type BuddyOrder = u16;

/// Returns the size of a buddy chunk of the given order.
pub(crate) const fn size_of_order(order: BuddyOrder) -> usize {
    (1 << order) * PAGE_SIZE
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
