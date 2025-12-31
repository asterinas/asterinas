// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::ops::{Deref, DerefMut};

use ostd::{
    sync::{SpinGuardian, SpinLockGuard, non_null::NonNullPtr},
    task::atomic_mode::{AsAtomicModeGuard, InAtomicMode},
    util::Either,
};

use crate::{
    BITS_PER_LAYER, SLOT_SIZE, XArray, XLockGuard,
    entry::NodeEntryRef,
    mark::{NoneMark, XMark},
    node::{Height, XNode},
};

/// A type representing the state of a [`Cursor`] or a [`CursorMut`].
///
/// Currently, there are two possible states:
///  - `Inactive`: The cursor is not positioned on any node.
///  - `AtNode`: The cursor is positioned on some node and holds a shared reference
///    to it.
///
/// A cursor never ends up on an interior node. In other words, when methods
/// of `Cursor` or `CursorMut` finish, the cursor will either not positioned on any node
/// or positioned on some leaf node.
#[derive(Default)]
enum CursorState<'a, P>
where
    P: NonNullPtr + Send + Sync,
{
    #[default]
    Inactive,
    AtNode {
        node: NodeEntryRef<'a, P>,
        operation_offset: u8,
    },
}

impl<'a, P: NonNullPtr + Send + Sync> CursorState<'a, P> {
    fn move_to(&mut self, node: NodeEntryRef<'a, P>, index: u64) {
        let operation_offset = node.entry_offset(index);
        *self = Self::AtNode {
            node,
            operation_offset,
        };
    }

    fn as_node(&self) -> Option<(&NodeEntryRef<'a, P>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((node, *operation_offset)),
            Self::Inactive => None,
        }
    }

    fn into_node(self) -> Option<(NodeEntryRef<'a, P>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((node, operation_offset)),
            Self::Inactive => None,
        }
    }

    fn is_at_node(&self) -> bool {
        match self {
            Self::AtNode { .. } => true,
            Self::Inactive => false,
        }
    }

    fn is_at_leaf(&self) -> bool {
        match self {
            Self::AtNode { node, .. } => node.is_leaf(),
            Self::Inactive => false,
        }
    }
}

/// A `Cursor` can traverse in the [`XArray`] by setting or increasing the
/// target index and can perform read-only operations to the target item.
///
/// Multiple `Cursor`s of the same `XArray` can exist simultaneously, and their existence
/// does not conflict with that of a [`CursorMut`].
///
/// A newly created `Cursor` can read all modifications that occurred before its creation.
/// Additionally, a `Cursor` can ensure it reads all modifications made before a specific
/// point by performing a [`Cursor::reset`] operation.
///
/// The typical way to obtain a `Cursor` instance is to call [`XArray::cursor`].
pub struct Cursor<'a, P, M = NoneMark>
where
    P: NonNullPtr + Send + Sync,
{
    /// The `XArray` where the cursor locates.
    xa: &'a XArray<P, M>,
    /// The target index of the cursor.
    index: u64,
    /// The atomic-mode guard that protects cursor operations.
    guard: &'a dyn InAtomicMode,
    /// The state of the cursor.
    state: CursorState<'a, P>,
}

impl<'a, P: NonNullPtr + Send + Sync, M> Cursor<'a, P, M> {
    /// Creates a `Cursor` to perform read-related operations in the `XArray`.
    pub(super) fn new<G: AsAtomicModeGuard>(
        xa: &'a XArray<P, M>,
        guard: &'a G,
        index: u64,
    ) -> Self {
        Self {
            xa,
            index,
            guard: guard.as_atomic_mode_guard(),
            state: CursorState::Inactive,
        }
    }

    /// Traverses from the root node to the leaf node according to the target index.
    ///
    /// This method will not create new nodes. If the cursor can not reach the target
    /// leaf node, the cursor will remain the inactive state.
    fn traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        let Some(head) = self.xa.head.read_with(self.guard) else {
            return;
        };

        let max_index = head.height().max_index();
        if max_index < self.index {
            return;
        }

        self.state.move_to(head, self.index);
        self.continue_traverse_to_target();
    }

    /// Traverses from an interior node to the leaf node according to the target index.
    ///
    /// This method will not create new nodes. If the cursor can not reach the target
    /// leaf node, the cursor will be reset to the inactive state.
    fn continue_traverse_to_target(&mut self) {
        while !self.state.is_at_leaf() {
            let (current_node, operation_offset) =
                core::mem::take(&mut self.state).into_node().unwrap();

            let Some(next_node) = current_node
                .deref_target()
                .entry_with(self.guard, operation_offset)
                .map(|operated_entry| operated_entry.left().unwrap())
            else {
                self.reset();
                return;
            };

            self.state.move_to(next_node, self.index);
        }
    }

    /**** Public ****/

    /// Loads the item at the target index.
    ///
    /// If the target item exists, this method will return a [`NonNullPtr::Ref`]
    /// that acts exactly like a `&'_ P` wrapped in `Some(_)`. Otherwises, it will
    /// return `None`.
    pub fn load(&mut self) -> Option<P::Ref<'a>> {
        self.traverse_to_target();
        let (node, operation_offset) = self.state.as_node()?;
        node.deref_target()
            .entry_with(self.guard, operation_offset)
            .and_then(|item_entry| item_entry.right())
    }

    /// Returns the target index of the cursor.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Resets the target index to `index`.
    pub fn reset_to(&mut self, index: u64) {
        self.reset();
        self.index = index;
    }

    /// Resets the cursor to the inactive state.
    pub fn reset(&mut self) {
        self.state = CursorState::Inactive;
    }

    /// Increases the target index of the cursor by one.
    ///
    /// Once increased, the cursor will be positioned on the corresponding leaf node
    /// if the leaf node exists.
    pub fn next(&mut self) {
        self.index = self.index.checked_add(1).unwrap();

        if !self.state.is_at_node() {
            return;
        }

        let (mut current_node, mut operation_offset) =
            core::mem::take(&mut self.state).into_node().unwrap();

        operation_offset += 1;
        while operation_offset == SLOT_SIZE as u8 {
            let Some(parent_node) = current_node.deref_target().parent(self.guard) else {
                self.reset();
                return;
            };

            operation_offset = current_node.offset_in_parent() + 1;
            current_node = parent_node;
        }

        self.state.move_to(current_node, self.index);
        self.continue_traverse_to_target();
    }

    /// Moves the cursor to the first present item at or after the current index.
    /// If found, updates the cursor's index and state, and returns the index.
    /// If not found, returns None.
    pub fn next_present(&mut self) -> Option<u64> {
        loop {
            self.traverse_to_target();

            let state = core::mem::take(&mut self.state);
            if let CursorState::AtNode {
                node,
                operation_offset,
            } = state
            {
                if node.is_leaf() {
                    // Check current slot
                    if node.entry_with(self.guard, operation_offset).is_some() {
                        self.state = CursorState::AtNode {
                            node,
                            operation_offset,
                        };
                        return Some(self.index);
                    }

                    // Check subsequent slots in this leaf
                    let mut off = operation_offset + 1;
                    while off < SLOT_SIZE as u8 {
                        if node.entry_with(self.guard, off).is_some() {
                            self.index += (off - operation_offset) as u64;
                            self.state = CursorState::AtNode {
                                node,
                                operation_offset: off,
                            };
                            return Some(self.index);
                        }
                        off += 1;
                    }

                    // Move to next leaf
                    let remaining_in_leaf = SLOT_SIZE as u64 - operation_offset as u64;
                    self.index = self.index.checked_add(remaining_in_leaf)?;
                    self.reset();
                } else {
                    // Should not happen if traverse_to_target works as expected (it stops at leaf or inactive).
                    // But if it stops at internal node, it means missing child.
                    // We should treat it as Inactive logic.
                    self.reset();
                }
            } else {
                // Inactive. Current index is empty.
                // Find next present from root.
                let head = self.xa.head.read_with(self.guard)?;
                let max_index = head.height().max_index();
                if self.index > max_index {
                    return None;
                }

                if let Some(next_idx) = self.find_next_from_root(self.index) {
                    self.index = next_idx;
                    // Loop will continue and traverse_to_target will succeed
                } else {
                    return None;
                }
            }
        }
    }

    fn find_next_from_root(&self, target: u64) -> Option<u64> {
        let head = self.xa.head.read_with(self.guard)?;
        self.find_next_in_node(&head, 0, target)
    }

    fn find_next_in_node(&self, node: &XNode<P>, node_base: u64, target: u64) -> Option<u64> {
        let height = node.height();
        let shift = (*height - 1) * BITS_PER_LAYER as u8;

        let start_offset = if target > node_base {
            ((target - node_base) >> shift) as u8
        } else {
            0
        };

        for off in start_offset..SLOT_SIZE as u8 {
            let child_base = node_base + ((off as u64) << shift);

            if let Some(entry) = node.entry_with(self.guard, off) {
                if *height == 1 {
                    // Leaf node.
                    if child_base >= target {
                        return Some(child_base);
                    }
                } else {
                    // Internal node.
                    let child = entry.left().unwrap();
                    let search_start = if off == start_offset {
                        target
                    } else {
                        child_base
                    };

                    if let Some(found) = self.find_next_in_node(&child, child_base, search_start) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }
}

impl<P: NonNullPtr + Send + Sync, M: Into<XMark>> Cursor<'_, P, M> {
    /// Checks whether the target item is marked with the input `mark`.
    ///
    /// If the target item does not exist, this method will also return false.
    pub fn is_marked(&mut self, mark: M) -> bool {
        self.traverse_to_target();
        self.state
            .as_node()
            .map(|(node, off)| node.is_marked(off, mark.into().index()))
            .unwrap_or(false)
    }
}

/// A `CursorMut` can traverse in the [`XArray`] by setting or increasing the
/// target index and can perform read-write operations to the target item.
///
/// An `XArray` can only have one `CursorMut` at a time, but a `CursorMut` can coexist
/// with multiple `Cursors` simultaneously.
///
/// The read-related operations of a `CursorMut` always retrieve up-to-date information.
///
/// The typical way to obtain a `CursorMut` instance is to call [`LockedXArray::cursor_mut`].
///
/// [`LockedXArray::cursor_mut`]: super::LockedXArray::cursor_mut
pub struct CursorMut<'a, P, M>(Cursor<'a, P, M>)
where
    P: NonNullPtr + Send + Sync;

impl<'a, P: NonNullPtr + Send + Sync, M> CursorMut<'a, P, M> {
    /// Creates a `CursorMut` to perform read- and write-related operations in the `XArray`.
    pub(super) fn new<G: SpinGuardian>(
        xa: &'a XArray<P, M>,
        guard: &'a SpinLockGuard<'a, (), G>,
        index: u64,
    ) -> Self {
        Self(Cursor::new(xa, guard, index))
    }

    /// Returns an `XLockGuard` that marks the `XArray` is locked.
    fn lock_guard(&self) -> XLockGuard<'_> {
        // Having a `CursorMut` means that the `XArray` is locked.
        XLockGuard(self.guard)
    }

    /// Increases the height of the `XArray` so that the `index`-th element can be stored.
    fn reserve(&self, index: u64) {
        if self.xa.head.read_with(self.guard).is_none() {
            let height = Height::from_index(index);
            let new_head = Arc::new(XNode::new_root(height));
            self.xa.head.update(Some(new_head));
            return;
        };

        loop {
            let head = self.xa.head.read_with(self.guard).unwrap();
            let height = head.height();
            if height.max_index() >= index {
                return;
            }

            let new_head = Arc::new(XNode::new_root(height.go_root()));
            new_head.set_entry(self.lock_guard(), 0, Some(Either::Left(head.clone())));

            self.xa.head.update(Some(new_head));
        }
    }

    /// Traverses from the root node to the leaf node according to the target index.
    ///
    /// This method will potentially create new nodes.
    fn expand_and_traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        let head = {
            self.reserve(self.index);
            self.xa.head.read_with(self.guard).unwrap()
        };

        self.0.state.move_to(head, self.0.index);
        self.continue_traverse_to_target_mut();
    }

    /// Traverses from an interior node to the leaf node according to the target index.
    ///
    /// This method will potentially create new nodes.
    fn continue_traverse_to_target_mut(&mut self) {
        while !self.state.is_at_leaf() {
            let (current_node, operation_offset) =
                core::mem::take(&mut self.state).into_node().unwrap();

            if current_node
                .entry_with(self.guard, operation_offset)
                .is_none()
            {
                let new_node = XNode::new(current_node.height().go_leaf(), operation_offset);
                let new_entry = Either::Left(Arc::new(new_node));
                current_node.set_entry(self.lock_guard(), operation_offset, Some(new_entry));
            }

            let next_node = current_node
                .deref_target()
                .entry_with(self.guard, operation_offset)
                .unwrap()
                .left()
                .unwrap();

            self.0.state.move_to(next_node, self.0.index);
        }
    }

    /**** Public ****/

    /// Stores a new `item` at the target index.
    pub fn store(&mut self, item: P) {
        self.expand_and_traverse_to_target();
        let (node, operation_offset) = self.state.as_node().unwrap();
        node.set_entry(
            self.lock_guard(),
            operation_offset,
            Some(Either::Right(item)),
        );
    }

    /// Removes the item at the target index.
    ///
    /// Returns the removed item if it previously exists.
    //
    // TODO: Remove the interior node once it becomes empty.
    pub fn remove(&mut self) -> Option<P::Ref<'a>> {
        self.traverse_to_target();
        self.state.as_node().and_then(|(node, off)| {
            let res = node
                .deref_target()
                .entry_with(self.guard, off)
                .and_then(|entry| entry.right());
            node.set_entry(self.lock_guard(), off, None);
            res
        })
    }
}

/// An error indicating that the mark cannot be set because the item does not exist.
#[derive(Debug)]
pub struct SetMarkError;

impl<P: NonNullPtr + Send + Sync, M: Into<XMark>> CursorMut<'_, P, M> {
    /// Sets the input `mark` for the item at the target index.
    ///  
    /// # Errors
    ///
    /// This method will fail with an error if the target item does not exist.
    pub fn set_mark(&mut self, mark: M) -> Result<(), SetMarkError> {
        self.traverse_to_target();
        self.state
            .as_node()
            .filter(|(node, off)| {
                node.entry_with(self.guard, *off)
                    .is_some_and(|entry| entry.is_right())
            })
            .map(|(node, off)| {
                let mark_index = mark.into().index();
                node.set_mark(self.lock_guard(), off, mark_index);
            })
            .ok_or(SetMarkError)
    }

    /// Unsets the input `mark` for the item at the target index.
    ///  
    /// # Errors
    ///
    /// This method will fail with an error if the target item does not exist.
    pub fn unset_mark(&mut self, mark: M) -> Result<(), SetMarkError> {
        self.traverse_to_target();
        self.state
            .as_node()
            .filter(|(node, off)| {
                node.entry_with(self.guard, *off)
                    .is_some_and(|entry| entry.is_right())
            })
            .map(|(node, off)| {
                let mark_index = mark.into().index();
                node.unset_mark(self.lock_guard(), off, mark_index);
            })
            .ok_or(SetMarkError)
    }
}

impl<'a, P: NonNullPtr + Send + Sync, M> Deref for CursorMut<'a, P, M> {
    type Target = Cursor<'a, P, M>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<P: NonNullPtr + Send + Sync, M> DerefMut for CursorMut<'_, P, M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
