// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{
    cmp::Ordering,
    ops::{Deref, DerefMut},
};

use ostd::{
    sync::{non_null::NonNullPtr, RcuOption},
    task::atomic_mode::InAtomicMode,
    util::Either,
};

use crate::{
    entry::{NodeEntry, NodeEntryRef, XEntry, XEntryRef},
    mark::{Mark, NUM_MARKS},
    XLockGuard, BITS_PER_LAYER, SLOT_MASK, SLOT_SIZE,
};

/// The height of an `XNode` within an `XArray`.
///
/// In an `XArray`, the head has the highest height, while the `XNode`s that
/// directly store items are at the lowest height, with a height value of 1.
/// Each level up from the bottom height increases the height number by 1.
/// The height of an `XArray` is the height of its head.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub(super) struct Height {
    height: u8,
}

impl Deref for Height {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.height
    }
}

impl DerefMut for Height {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.height
    }
}

impl PartialEq<u8> for Height {
    fn eq(&self, other: &u8) -> bool {
        self.height == *other
    }
}

impl PartialOrd<u8> for Height {
    fn partial_cmp(&self, other: &u8) -> Option<Ordering> {
        self.height.partial_cmp(other)
    }
}

impl Height {
    /// Creates a `Height` directly from a height value.
    pub(super) fn new(height: u8) -> Self {
        Self { height }
    }

    /// Creates a minimal `Height` that allows the `index`-th item to be stored.
    pub(super) fn from_index(index: u64) -> Self {
        let mut height = Height::new(1);
        while index > height.max_index() {
            *height += 1;
        }
        height
    }

    /// Goes up, which increases the height value by one.
    pub(super) fn go_root(&self) -> Self {
        Self::new(self.height + 1)
    }

    /// Goes down, which decreases the height value by one.
    pub(super) fn go_leaf(&self) -> Self {
        Self::new(self.height - 1)
    }

    fn height_shift(&self) -> u8 {
        (self.height - 1) * BITS_PER_LAYER as u8
    }

    /// Calculates the corresponding offset for the target index at
    /// the current height.
    pub(super) fn height_offset(&self, index: u64) -> u8 {
        ((index >> self.height_shift()) & SLOT_MASK as u64) as u8
    }

    /// Calculates the maximum index that can be represented in an `XArray`
    /// with the current height.
    pub(super) fn max_index(&self) -> u64 {
        ((SLOT_SIZE as u64) << self.height_shift()) - 1
    }
}

/// The `XNode` is the intermediate node in the tree-like structure of the `XArray`.
///
/// It contains `SLOT_SIZE` number of `XEntry`s, meaning it can accommodate up to
/// `SLOT_SIZE` child nodes. The `height` and `offset_in_parent` attributes of an
/// `XNode` are determined at initialization and remain unchanged thereafter.
pub(super) struct XNode<P>
where
    P: NonNullPtr + Send + Sync,
{
    /// The pointer that refers to the parent node.
    ///
    /// If the current node is the head node, its parent pointer will be `None`.
    parent: RcuOption<NodeEntry<P>>,
    /// The height of the subtree rooted at the current node.
    ///
    /// The height of a leaf node, which stores the user-given items, is 1.
    height: Height,
    /// This node is its parent's `offset_in_parent`-th child.
    ///
    /// This field will be zero if this node is the root, as the node will be
    /// the 0-th child of its parent once the height of `XArray` is increased.
    offset_in_parent: u8,
    /// The slots in which `XEntry`s are stored.
    ///
    /// The entries point to user-given items for leaf nodes and other `XNode`s for
    /// interior nodes.
    slots: [RcuOption<XEntry<P>>; SLOT_SIZE],
    /// The marks representing whether each slot is marked or not.
    ///
    /// Users can set mark or unset mark on user-given items, and a leaf
    /// node or an interior node is marked if and only if there is at least
    /// one marked item within the node.
    marks: [Mark; NUM_MARKS],
}

impl<P: NonNullPtr + Send + Sync> XNode<P> {
    pub(super) fn new_root(height: Height) -> Self {
        Self::new(height, 0)
    }

    pub(super) fn new(height: Height, offset: u8) -> Self {
        Self {
            parent: RcuOption::new_none(),
            height,
            offset_in_parent: offset,
            slots: [const { RcuOption::new_none() }; SLOT_SIZE],
            marks: [const { Mark::new_empty() }; NUM_MARKS],
        }
    }

    /// Gets the slot offset at the current `XNode` for the target index `target_index`.
    pub(super) fn entry_offset(&self, target_index: u64) -> u8 {
        self.height.height_offset(target_index)
    }

    pub(super) fn height(&self) -> Height {
        self.height
    }

    pub(super) fn parent<'a>(&'a self, guard: &'a dyn InAtomicMode) -> Option<NodeEntryRef<'a, P>> {
        let parent = self.parent.read_with(guard)?;
        Some(parent)
    }

    pub(super) fn offset_in_parent(&self) -> u8 {
        self.offset_in_parent
    }

    pub(super) fn entry_with<'a>(
        &'a self,
        guard: &'a dyn InAtomicMode,
        offset: u8,
    ) -> Option<XEntryRef<'a, P>> {
        self.slots[offset as usize].read_with(guard)
    }

    pub(super) fn is_marked(&self, offset: u8, mark: usize) -> bool {
        self.marks[mark].is_marked(offset)
    }

    pub(super) fn is_mark_clear(&self, mark: usize) -> bool {
        self.marks[mark].is_clear()
    }

    pub(super) fn is_leaf(&self) -> bool {
        self.height == 1
    }
}

impl<P: NonNullPtr + Send + Sync> XNode<P> {
    /// Sets the parent pointer of this node to the given `parent`.
    fn set_parent(&self, _guard: XLockGuard, parent: NodeEntry<P>) {
        self.parent.update(Some(parent));
    }

    /// Clears the parent pointers of this node and all its descendant nodes.
    ///
    /// This method should be invoked when the node is being removed from the tree.
    pub(super) fn clear_parent(&self, guard: XLockGuard) {
        self.parent.update(None);
        for child in self.slots.iter() {
            if let Some(node) = child.read_with(guard.0).and_then(|entry| entry.left()) {
                node.clear_parent(guard);
            }
        }
    }

    /// Sets the slot at the given `offset` to the given `entry`.
    ///
    /// If `entry` represents an item, the old marks at the same offset will be cleared.
    /// Otherwise, if `entry` represents a node, the marks at the same offset will be
    /// updated according to whether the new node contains marked items.
    ///
    /// This method will also propagate the updated marks to the ancestors.
    pub(super) fn set_entry(
        self: &Arc<Self>,
        guard: XLockGuard,
        offset: u8,
        entry: Option<XEntry<P>>,
    ) {
        let old_entry = self.slots[offset as usize].read_with(guard.0);
        if let Some(node) = old_entry.and_then(|entry| entry.left()) {
            node.clear_parent(guard);
        }

        let is_new_node = match &entry {
            Some(Either::Left(new_node)) => {
                new_node.set_parent(guard, self.clone());
                true
            }
            _ => false,
        };

        self.slots[offset as usize].update(entry);

        if is_new_node {
            self.update_mark(guard, offset);
        } else {
            for i in 0..NUM_MARKS {
                self.unset_mark(guard, offset, i);
            }
        }
    }

    /// Sets the input `mark` at the given `offset`.
    ///
    /// This method will also update the marks on the ancestors of this node
    /// if necessary to ensure that the marks on the ancestors are up to date.
    pub(super) fn set_mark(&self, guard: XLockGuard, offset: u8, mark: usize) {
        let changed = self.marks[mark].update(guard, offset, true);
        if changed {
            self.propagate_mark(guard, mark);
        }
    }

    /// Unsets the input `mark` at the given `offset`.
    ///
    /// This method will also update the marks on the ancestors of this node
    /// if necessary to ensure that the marks on the ancestors are up to date.
    pub(super) fn unset_mark(&self, guard: XLockGuard, offset: u8, mark: usize) {
        let changed = self.marks[mark].update(guard, offset, false);
        if changed {
            self.propagate_mark(guard, mark);
        }
    }

    /// Updates the mark at the given `offset`.
    ///
    /// This method does nothing if the slot at the given `offset` does not represent
    /// a node. It assumes the marks of the child node are up to date, and ensures
    /// the mark at the given `offset` is also up to date.
    ///
    /// This method will also update the marks on the ancestors of this node
    /// if necessary to ensure that the marks on the ancestors are up to date.
    fn update_mark(&self, guard: XLockGuard, offset: u8) {
        let entry = self.slots[offset as usize].read_with(guard.0);
        let Some(node) = entry.and_then(|entry| entry.left()) else {
            return;
        };

        for i in 0..NUM_MARKS {
            let changed = self.marks[i].update(guard, offset, !node.is_mark_clear(i));
            if changed {
                self.propagate_mark(guard, i);
            }
        }
    }

    /// Propagates the mark updates on this node to the ancestors.
    ///
    /// This method must be called after the marks are updated to ensure that the marks on the
    /// ancestors are up to date.
    fn propagate_mark(&self, guard: XLockGuard, mark: usize) {
        let Some(parent) = self.parent(guard.0) else {
            return;
        };

        let changed =
            parent.marks[mark].update(guard, self.offset_in_parent, !self.is_mark_clear(mark));
        if changed {
            parent.propagate_mark(guard, mark);
        }
    }
}
