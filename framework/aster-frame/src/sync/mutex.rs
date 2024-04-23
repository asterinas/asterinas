// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
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
    /// Create a new mutex.
    pub const fn new(val: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            queue: WaitQueue::new(),
            val: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquire the mutex.
    ///
    /// This method runs in a block way until the mutex can be acquired.
    pub fn lock(&self) -> MutexGuard<T> {
        self.queue.wait_until(|| self.try_lock())
    }

    /// Acquire the mutex through an [`Arc`].
    ///
    /// The method is similar to [`Self::lock`], but it doesn't have the requirement
    /// for compile-time checked lifetimes of the mutex guard.
    pub fn lock_arc(self: &Arc<Self>) -> ArcMutexGuard<T> {
        self.queue.wait_until(|| self.try_lock_arc())
    }

    /// Try Acquire the mutex immedidately.
    pub fn try_lock(&self) -> Option<MutexGuard<T>> {
        // Cannot be reduced to `then_some`, or the possible dropping of the temporary
        // guard will cause an unexpected unlock.
        self.acquire_lock().then_some(MutexGuard { mutex: self })
    }

    /// Try acquire the mutex through an [`Arc`].
    ///
    /// The method is similar to [`Self::try_lock`], but it doesn't have the requirement
    /// for compile-time checked lifetimes of the mutex guard.
    pub fn try_lock_arc(self: &Arc<Self>) -> Option<ArcMutexGuard<T>> {
        self.acquire_lock().then(|| ArcMutexGuard {
            mutex: self.clone(),
        })
    }

    /// Release the mutex and wake up one thread which is blocked on this mutex.
    fn unlock(&self) {
        self.release_lock();
        self.queue.wake_one();
    }

    fn acquire_lock(&self) -> bool {
        self.lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn release_lock(&self) {
        self.lock.store(false, Ordering::Release);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

#[clippy::has_significant_drop]
pub struct MutexGuard_<T: ?Sized, R: Deref<Target = Mutex<T>>> {
    mutex: R,
}

pub type MutexGuard<'a, T> = MutexGuard_<T, &'a Mutex<T>>;
pub type ArcMutexGuard<T> = MutexGuard_<T, Arc<Mutex<T>>>;

impl<T: ?Sized, R: Deref<Target = Mutex<T>>> Deref for MutexGuard_<T, R> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = Mutex<T>>> DerefMut for MutexGuard_<T, R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = Mutex<T>>> Drop for MutexGuard_<T, R> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}

impl<T: ?Sized + fmt::Debug, R: Deref<Target = Mutex<T>>> fmt::Debug for MutexGuard_<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized, R: Deref<Target = Mutex<T>>> !Send for MutexGuard_<T, R> {}

unsafe impl<T: ?Sized + Sync, R: Deref<Target = Mutex<T>> + Sync> Sync for MutexGuard_<T, R> {}
