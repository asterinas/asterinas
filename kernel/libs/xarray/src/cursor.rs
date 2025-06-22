// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::ops::{Deref, DerefMut};

use ostd::{
    sync::{non_null::NonNullPtr, SpinGuardian, SpinLockGuard},
    task::atomic_mode::{AsAtomicModeGuard, InAtomicMode},
    util::Either,
};

use crate::{
    entry::NodeEntryRef,
    mark::{NoneMark, XMark},
    node::{Height, XNode},
    XArray, XLockGuard, SLOT_SIZE,
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
enum CursorState<'a, P>
where
    P: NonNullPtr + Send + Sync,
{
    Inactive,
    AtNode {
        node: NodeEntryRef<'a, P>,
        operation_offset: u8,
    },
}

impl<P: NonNullPtr + Send + Sync> Default for CursorState<'_, P> {
    fn default() -> Self {
        Self::Inactive
    }
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
    fn lock_guard(&self) -> XLockGuard {
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
