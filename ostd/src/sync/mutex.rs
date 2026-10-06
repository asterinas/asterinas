// SPDX-License-Identifier: MPL-2.0

use core::{
    cell::UnsafeCell,
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use super::WaitQueue;

/// A mutex with waitqueue.
pub struct Mutex<T: ?Sized> {
    lock: AtomicBool,
    queue: WaitQueue,
    val: UnsafeCell<T>,
}

impl<T> Mutex<T> {
    /// Creates a new mutex.
    pub const fn new(val: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            queue: WaitQueue::new(),
            val: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquires the mutex.
    ///
    /// This method runs in a block way until the mutex can be acquired.
    #[track_caller]
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.queue.wait_until(|| self.try_lock())
    }

    /// Tries to acquire the mutex immediately.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.acquire_lock()
            // We cannot use `then_some`. Otherwise, the possible dropping of the temporary
            // guard will cause an unexpected unlock.
            // SAFETY: The lock is successfully acquired when creating the guard.
            .then(|| unsafe { MutexGuard::new(self) })
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// This method is zero-cost: By holding a mutable reference to the lock, the compiler has
    /// already statically guaranteed that access to the data is exclusive.
    pub fn get_mut(&mut self) -> &mut T {
        self.val.get_mut()
    }

    /// Releases the mutex and wake up one thread which is blocked on this mutex.
    ///
    /// # Safety
    ///
    /// The caller must ensure that it has acquired the lock.
    unsafe fn unlock(&self) {
        // SAFETY: The safety is upheld by the caller.
        unsafe { self.release_lock() };
        self.queue.wake_one();
    }

    fn acquire_lock(&self) -> bool {
        self.lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// # Safety
    ///
    /// The caller must ensure that it has acquired the lock.
    unsafe fn release_lock(&self) {
        self.lock.store(false, Ordering::Release);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

// SAFETY: Only a single lock holder is permitted to access the inner data of `Mutex`.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

/// A guard that provides exclusive access to the data protected by a [`Mutex`].
#[clippy::has_significant_drop]
#[must_use]
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
}

impl<'a, T: ?Sized> MutexGuard<'a, T> {
    /// # Safety
    ///
    /// The caller must ensure that the given reference of [`Mutex`] lock has been successfully
    /// acquired in the current context. When the created [`MutexGuard`] is dropped, it will unlock
    /// the [`Mutex`].
    unsafe fn new(mutex: &'a Mutex<T>) -> Self {
        MutexGuard { mutex }
    }

    /// Returns the [`Mutex`] associated with this guard.
    pub fn get_lock(guard: &Self) -> &'a Mutex<T> {
        guard.mutex
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The mutex is locked, which provides exclusive access to the data.
        unsafe { &*self.mutex.val.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The mutex is locked, which provides exclusive access to the data.
        unsafe { &mut *self.mutex.val.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: The mutex is locked.
        unsafe { self.mutex.unlock() };
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized> !Send for MutexGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for MutexGuard<'_, T> {}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    // A regression test for a bug fixed in [#1279](https://github.com/asterinas/asterinas/pull/1279).
    #[ktest]
    fn try_lock_does_not_unlock() {
        let lock = Mutex::new(0);
        assert!(!lock.lock.load(Ordering::Relaxed));

        // A successful lock
        let guard1 = lock.lock();
        assert!(lock.lock.load(Ordering::Relaxed));

        // A failed `try_lock` won't drop the lock
        assert!(lock.try_lock().is_none());
        assert!(lock.lock.load(Ordering::Relaxed));

        // Ensure the lock is held until here
        drop(guard1);
    }
}
