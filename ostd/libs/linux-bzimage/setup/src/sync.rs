// SPDX-License-Identifier: MPL-2.0

//! Synchronization primitives.

use core::cell::{RefCell, RefMut};

/// A mutex.
pub struct Mutex<T>(RefCell<T>);

// SAFETY: We're single-threaded.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Sync> Sync for Mutex<T> {}

/// A mutex guard.
type MutexGuard<'a, T> = RefMut<'a, T>;

impl<T> Mutex<T> {
    /// Creates a new mutex.
    pub const fn new(data: T) -> Self {
        Self(RefCell::new(data))
    }

    /// Locks the mutex.
    pub fn lock(&self) -> MutexGuard<T> {
        self.0.borrow_mut()
    }
}
