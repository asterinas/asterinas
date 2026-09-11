// SPDX-License-Identifier: MPL-2.0

//! Synchronization primitives.

use core::cell::{RefCell, RefMut};

/// A mutex.
pub(crate) struct Mutex<T>(RefCell<T>);

// SAFETY: We're single-threaded.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Sync> Sync for Mutex<T> {}

/// A mutex guard.
type MutexGuard<'a, T> = RefMut<'a, T>;

impl<T> Mutex<T> {
    /// Creates a new mutex.
    pub(crate) const fn new(data: T) -> Self {
        Self(RefCell::new(data))
    }

    /// Locks the mutex.
    pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
        self.0.borrow_mut()
    }
}
