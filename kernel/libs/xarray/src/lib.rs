// SPDX-License-Identifier: MPL-2.0

//! This crate introduces a RCU-based [`XArray`] implementation.
//!
//! `XArray` is an abstract data type functioning like an expansive array of items
//! where each item is a [`NonNullPtr`], such as `Arc<T>` or `Box<T>`. It facilitates
//! efficient sequential access to adjacent entries, supporting multiple concurrent reads
//! and exclusively allowing one write operation at a time.
//!
//! In addition to directly manipulating the `XArray`, users can typically achieve more
//! flexible operations by creating a [`Cursor`]/[`CursorMut`] within the `XArray`. Since the
//! `XArray` enforces a single write operation at any given time, performing write operations
//! requires first acquiring a [`LockedXArray`] by calling its `lock` method.
//!
//! `XArray` also provides a convenient way to mark individual items (see [`XMark`]).
//!
//! # Example
//!
//! ```
//! use alloc::sync::Arc;
//!
//! use crare::rcu_xarray::*;
//! use crate::task::disable_preempt;
//!
//! let xarray_arc: XArray<Arc<i32>> = XArray::new();
//! let value = Arc::new(10);
//! xarray_arc.lock().store(10, value);
//!
//! let guard = disable_preempt();
//! assert_eq!(*xarray_arc.load(&guard, 10).unwrap().as_ref(), 10);
//!
//! // Usage of the cursor
//!
//! let locked_xarray = xarray_arc.lock();
//! let cursor_mut = locked_xarray.cursor_mut(100);
//!
//! let value = Arc::new(100);
//! cursor_mut.store(value);
//! assert_eq!(cursor_mut.load(10).unwrap().as_ref(), 100);
//! let cursor = xarray_arc.cursor(&guard, 100);
//! assert_eq!(cursor.load(10).unwrap().as_ref(), 100);
//! ```
//!
//! # Background
//!
//! The XArray concept was originally introduced by Linux, which keeps the data structure of
//! [Linux Radix Trees](https://lwn.net/Articles/175432/).

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use core::marker::PhantomData;

pub use cursor::{Cursor, CursorMut, SetMarkError};
use entry::NodeEntry;
use mark::NoneMark;
pub use mark::XMark;
use ostd::{
    sync::{
        non_null::NonNullPtr, LocalIrqDisabled, PreemptDisabled, RcuOption, SpinGuardian, SpinLock,
        SpinLockGuard,
    },
    task::atomic_mode::{AsAtomicModeGuard, InAtomicMode},
};
pub use range::Range;

mod cursor;
mod entry;
mod mark;
mod node;
mod range;

#[cfg(ktest)]
mod test;

const BITS_PER_LAYER: usize = 6;
const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
const SLOT_MASK: usize = SLOT_SIZE - 1;

/// A RCU-based `XArray` implementation.
///
/// `XArray` is used to store [`NonNullPtr`], with the additional requirement that user-stored
/// pointers must have a minimum alignment of 2 bytes.
///
/// `XArray` is RCU-based, which means:
/// - Multiple concurrent readers are permitted.
/// - Only one writer is allowed at a time.
/// - Simultaneous read operations are allowed while writing.
/// - Readers may see stale data (see [`Cursor`] and [`CursorMut`] for more information).
///
/// Interaction with `XArray` is generally through `Cursor` and `CursorMut`. Similar to
/// XArray's read-write properties, multiple `Cursor`s may coexist (shared read access) and
/// only one `CursorMut` may exist at a time (exclusive write access).
///
/// To create a `Cursor`, users can invoke [`XArray::cursor`] with an atomic-guard.
/// To create a `CursorMut`, users need to call [`XArray::lock`] or [`XArray::lock_irq_disabled`]
/// first to obtain a [`LockedXArray`] first.
///
/// `XArray` enables marking of individual items for user convenience. Items can have up to three
/// distinct marks by default, with each mark independently maintained. Users can use self-defined
/// types as marks by implementing the `From<Type>` trait for [`XMark`]. Marking is also applicable
/// to internal nodes, indicating marked descendant nodes, though such marking is not transparent
/// to users.
pub struct XArray<P, M = NoneMark>
where
    P: NonNullPtr + Send + Sync,
{
    head: RcuOption<NodeEntry<P>>,
    xlock: SpinLock<()>,
    _marker: PhantomData<M>,
}

/// A type that marks the [`XArray`] is locked.
#[derive(Clone, Copy)]
struct XLockGuard<'a>(&'a dyn InAtomicMode);

impl<P: NonNullPtr + Send + Sync, M> Default for XArray<P, M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: NonNullPtr + Send + Sync, M> XArray<P, M> {
    /// Makes a new, empty `XArray`.
    pub const fn new() -> Self {
        Self {
            head: RcuOption::new_none(),
            xlock: SpinLock::new(()),
            _marker: PhantomData,
        }
    }

    /// Acquires the lock to perform mutable operations.
    pub fn lock(&self) -> LockedXArray<P, M> {
        LockedXArray {
            xa: self,
            guard: self.xlock.lock(),
            _marker: PhantomData,
        }
    }

    /// Acquires the lock with local IRQs disabled to perform mutable operations.
    pub fn lock_irq_disabled(&self) -> LockedXArray<P, M, LocalIrqDisabled> {
        LockedXArray {
            xa: self,
            guard: self.xlock.disable_irq().lock(),
            _marker: PhantomData,
        }
    }

    /// Creates a [`Cursor`] to perform read-related operations.
    pub fn cursor<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        index: u64,
    ) -> Cursor<'a, P, M> {
        Cursor::new(self, guard, index)
    }

    /// Creates a [`Range`] to immutably iterated over the specified `range`.
    pub fn range<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        range: core::ops::Range<u64>,
    ) -> Range<'a, P, M> {
        let cursor = self.cursor(guard, range.start);
        Range::new(cursor, range.end)
    }

    /// Loads the `index`-th item.
    ///
    /// If the target item exists, it will be returned with `Some(_)`,
    /// otherwise, `None` will be returned.
    pub fn load<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        index: u64,
    ) -> Option<P::Ref<'a>> {
        let mut cursor = self.cursor(guard, index);
        cursor.load()
    }
}

impl<P: NonNullPtr + Sync + Send, M> Drop for XArray<P, M> {
    fn drop(&mut self) {
        self.lock().clear();
    }
}

/// The locked [`XArray`] which obtains its inner spinlock.
///
/// The locked `XArray` is able to create `CursorMut` and do mutable operations.
/// There can only be one locked `XArray` at the same time.
pub struct LockedXArray<'a, P, M = NoneMark, G = PreemptDisabled>
where
    P: NonNullPtr + Send + Sync,
    G: SpinGuardian,
{
    xa: &'a XArray<P, M>,
    guard: SpinLockGuard<'a, (), G>,
    _marker: PhantomData<(P, M)>,
}

impl<P: NonNullPtr + Send + Sync, M, G: SpinGuardian> LockedXArray<'_, P, M, G> {
    /// Clears the corresponding [`XArray`].
    pub fn clear(&mut self) {
        if let Some(head) = self.xa.head.read_with(&self.guard) {
            // Having a `LockedXArray` means that the `XArray` is locked.
            head.clear_parent(XLockGuard(self.guard.as_atomic_mode_guard()));
        }

        self.xa.head.update(None);
    }

    /// Creates a [`CursorMut`] to perform read- and write-related operations.
    pub fn cursor_mut(&mut self, index: u64) -> CursorMut<'_, P, M> {
        CursorMut::new(self.xa, &self.guard, index)
    }

    /// Stores the provided item at the target index.
    pub fn store(&mut self, index: u64, item: P) {
        let mut cursor = self.cursor_mut(index);
        cursor.store(item)
    }

    /// Removes the item at the target index.
    ///
    /// Returns the removed item if some item was previously stored in the same position.
    pub fn remove(&mut self, index: u64) -> Option<P::Ref<'_>> {
        let mut cursor = self.cursor_mut(index);
        cursor.remove()
    }

    /// Creates a [`Cursor`] to perform read-related operations.
    pub fn cursor(&self, index: u64) -> Cursor<'_, P, M> {
        Cursor::new(self.xa, &self.guard, index)
    }

    /// Creates a [`Range`] to immutably iterated over the specified `range`.
    pub fn range(&self, range: core::ops::Range<u64>) -> Range<'_, P, M> {
        let cursor = self.cursor(range.start);
        Range::new(cursor, range.end)
    }

    /// Loads the `index`-th item.
    ///
    /// If the target item exists, it will be returned with `Some(_)`, otherwise, `None` will be
    /// returned.
    pub fn load(&self, index: u64) -> Option<P::Ref<'_>> {
        let mut cursor = self.cursor(index);
        cursor.load()
    }
}
