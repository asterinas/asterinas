use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use super::WaitQueue;

/// A read/write lock based on blocking, which is named `RwMutex`.
///
/// ```
/// Now, the mutex's layout is simply like:
/// bit:       63      |        62 ~ 0
/// use:  writer mutex | reader mutex & numbers
/// ```
pub struct RwMutex<T> {
    val: UnsafeCell<T>,
    lock: AtomicUsize,
    queue: WaitQueue,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const MAX_READER: usize = WRITER >> 1;

impl<T> RwMutex<T> {
    /// Creates a new `RwMutex`.
    pub fn new(val: T) -> Self {
        Self {
            val: UnsafeCell::new(val),
            lock: AtomicUsize::new(0),
            queue: WaitQueue::new(),
        }
    }

    /// Acquire a read mutex, and if there is a writer, this thread will sleep in the wait queue.
    pub fn read(&self) -> RwMutexReadGuard<T> {
        self.queue.wait_until(|| self.try_read())
    }

    /// Acquire a write mutex, and if there is another writer or other readers, this thread will sleep in the wait queue.
    pub fn write(&self) -> RwMutexWriteGuard<T> {
        self.queue.wait_until(|| self.try_write())
    }

    /// Try acquire a read mutex and return immediately if it fails.
    pub fn try_read(&self) -> Option<RwMutexReadGuard<T>> {
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER) == 0 {
            Some(RwMutexReadGuard { inner: &self })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Try acquire a write mutex and return immediately if it fails.
    pub fn try_write(&self) -> Option<RwMutexWriteGuard<T>> {
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwMutexWriteGuard { inner: &self })
        } else {
            None
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for RwMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

/// Because there can be more than one readers to get the T's immutable ref,
/// so T must be Sync to guarantee the sharing safety.
unsafe impl<T: Send> Send for RwMutex<T> {}
unsafe impl<T: Send + Sync> Sync for RwMutex<T> {}

impl<'a, T> !Send for RwMutexWriteGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwMutexWriteGuard<'_, T> {}

impl<'a, T> !Send for RwMutexReadGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwMutexReadGuard<'_, T> {}

/// The guards of `RwMutex`.
pub struct RwMutexReadGuard<'a, T> {
    inner: &'a RwMutex<T>,
}

/// Upgrade a read mutex to a write mutex.
///
/// This method first release the old read mutex and then aquire a new write mutex.
/// So it may sleep while acquireing the write mutex.
impl<'a, T> RwMutexReadGuard<'a, T> {
    pub fn upgrade(self) -> RwMutexWriteGuard<'a, T> {
        let inner = self.inner;
        drop(self);
        inner.write()
    }
}

impl<'a, T> Deref for RwMutexReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

/// When there are no readers, wake up a waiting writer.
impl<'a, T> Drop for RwMutexReadGuard<'a, T> {
    fn drop(&mut self) {
        if self.inner.lock.fetch_sub(READER, Release) == 1 {
            self.inner.queue.wake_one();
        }
    }
}

pub struct RwMutexWriteGuard<'a, T> {
    inner: &'a RwMutex<T>,
}

impl<'a, T> RwMutexWriteGuard<'a, T> {
    pub fn downgrade(self) -> RwMutexReadGuard<'a, T> {
        self.inner.lock.fetch_add(READER, Acquire);
        let inner = self.inner;
        drop(self);
        RwMutexReadGuard { inner }
    }
}

impl<'a, T> Deref for RwMutexWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> DerefMut for RwMutexWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

/// When the current writer releases, wake up all the sleeping threads.
impl<'a, T> Drop for RwMutexWriteGuard<'a, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!(WRITER), Release);

        // All awakened threads may include readers and writers.
        // Thanks to the `wait_until` method, either all readers
        // continue to execute or one writer continues to execute.
        self.inner.queue.wake_all();
    }
}
