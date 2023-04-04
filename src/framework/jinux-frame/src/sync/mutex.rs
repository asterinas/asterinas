use super::spin::{SpinLock, SpinLockGuard};
use core::ops::{Deref, DerefMut};

use core::fmt;

pub struct Mutex<T> {
    inner: SpinLock<T>,
}

impl<T> Mutex<T> {
    #[inline(always)]
    pub const fn new(val: T) -> Self {
        Self {
            inner: SpinLock::new(val),
        }
    }

    pub fn lock(&self) -> MutexGuard<T> {
        MutexGuard {
            lock: self.inner.lock(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

pub struct MutexGuard<'a, T> {
    lock: SpinLockGuard<'a, T>,
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.lock.deref()
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.lock.deref_mut()
    }
}

impl<'a, T: fmt::Debug> fmt::Debug for MutexGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<'a, T> !Send for MutexGuard<'a, T> {}

unsafe impl<T: Sync> Sync for MutexGuard<'_, T> {}
