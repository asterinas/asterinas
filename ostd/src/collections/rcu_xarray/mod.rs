// SPDX-License-Identifier: MPL-2.0

//! This module introduces the RCU-based xarray.

use core::marker::PhantomData;

pub use cursor::{Cursor, CursorMut};
use entry::XEntry;
use mark::NoneMark;
pub use mark::XMark;
pub use range::Range;

use crate::{
    sync::{
        non_null::NonNullPtr, LocalIrqDisabled, PreemptDisabled, RcuOption, SpinGuardian, SpinLock,
        SpinLockGuard,
    },
    task::atomic_mode::AsAtomicModeGuard,
};

mod cursor;
mod entry;
mod mark;
mod node;
mod range;

pub(super) const BITS_PER_LAYER: usize = 6;
pub(super) const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
pub(super) const SLOT_MASK: usize = SLOT_SIZE - 1;

/// `XArray` is an abstract data type functioning like an expansive array of items
/// where each item must be an 8-byte object, such as `Arc<T>` or `Box<T>`.
///
/// User-stored pointers must have a minimum alignment of 4 bytes. `XArray` facilitates
/// efficient sequential access to adjacent entries, supporting multiple concurrent reads
/// and exclusively allowing one write operation at a time.
///
/// The XArray is RCU-based, which means:
/// - Multiple concurrent readers are permitted.
/// - Only one writer is allowed at a time.
/// - Allows simultaneous read operations while writing.
/// - Readers may see stale data (see [`Cursor`] and [`CursorMut`] for more information).
///
/// Interaction with `XArray` is mediated through `Cursor` and `CursorMut`. Similar to
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
///
/// # Example
///
/// ```
/// use alloc::sync::Arc;
///
/// use crare::rcu_xarray::*;
/// use crate::task::disable_preempt;
///
/// let xarray_arc: XArray<Arc<i32>> = XArray::new();
/// let value = Arc::new(10);
/// xarray_arc.lock().store(10, value);
///
/// let guard = disable_preempt();
/// assert_eq!(*xarray_arc.load(&guard, 10).unwrap().as_ref(), 10);
///
/// // Usage of the cursor
///
/// let locked_xarray = xarray_arc.lock();
/// let cursor_mut = locked_xarray.cursor_mut(100);
///
/// let value = Arc::new(100);
/// cursor_mut.store(value);
/// assert_eq!(cursor_mut.load(10).unwrap().as_ref(), 100);
/// let cursor = xarray_arc.cursor(&guard, 100);
/// assert_eq!(cursor.load(10).unwrap().as_ref(), 100);
/// ```
///
/// The XArray concept was originally introduced by Linux, which keeps the data structure of
/// [Linux Radix Trees](https://lwn.net/Articles/175432/).
#[repr(C)]
pub struct XArray<P, M = NoneMark>
where
    P: NonNullPtr + Send + Sync,
    M: Into<XMark>,
{
    head: RcuOption<XEntry<P>>,
    xlock: SpinLock<()>,
    _marker: PhantomData<M>,
}

impl<P: NonNullPtr + Send + Sync, M: Into<XMark>> Default for XArray<P, M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: NonNullPtr + Send + Sync, M: Into<XMark>> XArray<P, M> {
    /// Makes a new, empty `XArray`.
    pub const fn new() -> Self {
        Self {
            head: RcuOption::new_none(),
            xlock: SpinLock::new(()),
            _marker: PhantomData,
        }
    }

    /// Creates a [`Cursor`] to perform read-related operations in the `XArray`.
    pub fn cursor<'a>(&'a self, guard: &'a dyn AsAtomicModeGuard, index: u64) -> Cursor<'a, P, M> {
        Cursor::new(self, guard, index)
    }

    /// Creates a [`Range`] which can be immutably iterated over the indexes
    /// corresponding to the specified `range`.
    pub fn range<'a>(
        &'a self,
        guard: &'a dyn AsAtomicModeGuard,
        range: core::ops::Range<u64>,
    ) -> Range<'a, P, M> {
        let cursor = self.cursor(guard, range.start);
        Range::new(cursor, range.end)
    }

    /// Acquire the `xlock` in the `XArray` and returns [`LockedXArray`] for
    /// mutable operations on this `XArray`.
    pub fn lock(&self) -> LockedXArray<P, M> {
        LockedXArray {
            xa: self,
            guard: self.xlock.lock(),
            _marker: PhantomData,
        }
    }

    /// Acquire the spinlock in the `XArray` with disabling local IRQs
    /// and returns [`LockedXArray`] for mutable operations on this `XArray`.
    pub fn lock_irq_disabled(&self) -> LockedXArray<P, M, LocalIrqDisabled> {
        LockedXArray {
            xa: self,
            guard: self.xlock.disable_irq().lock(),
            _marker: PhantomData,
        }
    }

    /// Loads the `index`-th item.
    ///
    /// If the target item exists, it will be returned with `Some(_)`,
    /// otherwise, `None` will be returned.
    pub fn load<'a>(&'a self, guard: &'a dyn AsAtomicModeGuard, index: u64) -> Option<P::Ref<'a>> {
        let mut cursor = self.cursor(guard, index);
        cursor.load()
    }
}

/// The locked [`XArray`] which obtains its inner spinlock.
///
/// The locked `XArray` is able to create `CursorMut` and do mutable operations.
/// There can only be one locked `XArray` at the same time.
pub struct LockedXArray<'a, P, M, G = PreemptDisabled>
where
    P: NonNullPtr + Send + Sync,
    M: Into<XMark>,
    G: SpinGuardian,
{
    xa: &'a XArray<P, M>,
    guard: SpinLockGuard<'a, (), G>,
    _marker: PhantomData<(P, M)>,
}

impl<P, M, G> LockedXArray<'_, P, M, G>
where
    P: NonNullPtr + Send + Sync,
    M: Into<XMark>,
    G: SpinGuardian,
{
    /// Creates a [`CursorMut`] to perform read- and write-related operations
    /// in the [`XArray`].
    pub fn cursor_mut(&mut self, index: u64) -> cursor::CursorMut<'_, P, M> {
        cursor::CursorMut::new(self.xa, &self.guard, index)
    }

    /// Stores the provided item in the [`XArray`] at the target index.
    pub fn store(&mut self, index: u64, item: P) {
        let mut cursor = self.cursor_mut(index);
        cursor.store(item)
    }

    /// Removes the item in the [`XArray`] at the target index.
    ///
    /// Returns the removed item if some item was previously stored in the same position.
    pub fn remove(&mut self, index: u64) -> Option<P::Ref<'_>> {
        let mut cursor = self.cursor_mut(index);
        cursor.remove()
    }

    /// Clears the corresponding [`XArray`].
    pub fn clear(&mut self) {
        self.xa.head.update(None);
    }

    /// Creates a [`Cursor`] to perform read-related operations in the `XArray`.
    pub fn cursor(&self, index: u64) -> Cursor<'_, P, M> {
        Cursor::new(self.xa, &self.guard, index)
    }

    /// Creates a [`Range`] which can be immutably iterated over the indexes corresponding to the
    /// specified `range`.
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
